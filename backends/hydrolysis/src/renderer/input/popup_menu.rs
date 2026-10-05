// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::widgets::controls::button::ListRowChrome;
use core::ops::RangeInclusive;
use core::time::Duration;
use nami::Signal;
use waterui::form::Calendar;
use waterui::shape::{FixedRoundedRectangle, RoundedRectangle, ShapeExt as _};
use waterui::theme::color::Surface;
use waterui_backend_core::widget::{ButtonMetrics, InteractionStyle, PickerMetrics};
use waterui_controls::button::ButtonStyle;
use waterui_controls::label::LabelDisplayMode;
use waterui_controls::menu::Shortcut;
use waterui_controls::{Stepper, button, stepper::stepper};
use waterui_core::metadata::{Metadata, MetadataKey};
use waterui_core::{SignalExt as _, id::Id};
use waterui_form::picker::date::{Date, DatePickerType, DateTime};
use waterui_layout::frame::Frame;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::spacer::spacer;
use waterui_layout::stack::hstack;
use waterui_text::text;

/// Backend-private presentation service for transient popup windows.
///
/// Keeping popup presentation separate from the public
/// [`waterui::window::WindowManager`] lets
/// the desktop runner mount menus without activating them while ordinary
/// application windows retain their normal focus behavior.
#[derive(Clone)]
pub struct PopupWindowManager(Rc<dyn Fn(Window)>);

impl PopupWindowManager {
    pub(crate) fn new(show: impl Fn(Window) + 'static) -> Self {
        Self(Rc::new(show))
    }

    /// Mounts `window` re-rooted in `env` — the environment of the view that
    /// opened the popup (water-rs/hydrolysis#140), so its `.state(&value)`
    /// and `.with(...)` overlays reach the item actions' extractors exactly
    /// as they do for inline views.
    pub(crate) fn show(&self, window: Window, env: &Environment) {
        (self.0)(window_in_opening_environment(window, env));
    }
}

/// Rebuilds `window`'s content inside `env` wholesale: the retained tree the
/// runner captures for the popup then inherits the opening view's
/// environment — `.state(&value)`, themes, `.with(...)` values — instead of
/// the runtime's bare root. `Metadata<Environment>` replaces the environment
/// for the subtree it wraps, so values the popup itself pushes (`.with(...)`,
/// `.state(&...)` inside its own content) still layer on top. The popup's
/// frame origin becomes the subtree's `HydrolysisWindowOrigin`, the same
/// value the input dispatcher installs for a root window, so a nested popup
/// opened from this window anchors to it.
pub fn window_in_opening_environment(mut window: Window, env: &Environment) -> Window {
    let frame = window.frame.snapshot();
    let content_env = env.extending(HydrolysisWindowOrigin {
        x: frame.x(),
        y: frame.y(),
    });
    let content = window.content;
    window.content = AnyViewBuilder::new(move || {
        AnyView::new(Metadata::new(content.build(), content_env.clone()))
    });
    window
}

#[derive(Clone)]
pub struct ContextMenuTarget {
    pub(crate) bounds: kurbo::Rect,
    pub(crate) depth: usize,
    pub(crate) order: usize,
    pub(crate) items: nami::Computed<Vec<ResolvedMenuItem>>,
    /// The environment of the view that declared the menu — its popup opens
    /// inside it, so `.state(&value)` overlays reach the item actions.
    pub(crate) env: Environment,
    /// The accessory's dismiss-request counter: every change closes the open
    /// menu (water-rs/hydrolysis#200, water-rs/waterui#1245).
    pub(crate) dismiss_requests: nami::Computed<i32>,
    /// The view the owning node lends the open presentation to lift over the
    /// dimmed backdrop at `bounds`; `None` content means the source view stays
    /// lit through the backdrop's hole instead.
    pub(crate) preview: Rc<RefCell<Option<RetainedSubview>>>,
    /// The interactive accessory the owning node lends the open presentation
    /// to anchor to the lifted preview. Returned to the slot on close.
    pub accessory: Rc<RefCell<Option<RetainedSubview>>>,
}

#[derive(Clone)]
pub enum PopupMenuNode {
    Command {
        label: SemanticLabel,
        plain_label: String,
        action: SharedAction<()>,
        /// The live disabled signal — the row draws the snapshot it was built
        /// with; the window's shortcut table reads it at dispatch time so a
        /// command enabled while its menu is mounted fires again.
        disabled: nami::Computed<bool>,
        /// The command's keyboard chord: drawn as the row's trailing hint and
        /// registered on the window's shortcut table while the menu is open.
        shortcut: Option<Shortcut>,
        /// A secondary line under the label, drawn in the muted supporting
        /// text style; the row grows to fit it.
        subtitle: Option<Str>,
    },
    Divider,
    Menu {
        label: SemanticLabel,
        plain_label: String,
        items: Vec<Self>,
    },
}

#[derive(Clone)]
pub struct PopupMenuStateGroup(pub Rc<RefCell<Vec<Binding<WindowState>>>>);

#[derive(Default)]
pub struct PopupMenuState {
    pub active_popup_menu_group: Option<PopupMenuStateGroup>,
    /// The drawn presentation around an open `.context_menu` popup — dimmed
    /// backdrop, lifted preview and anchored accessory. `None` when the active
    /// menu was not opened from a context-menu target or carries no preview.
    pub(crate) context_menu_presentation: Option<ContextMenuPresentation>,
    /// Open/closed handles for the picker menus currently in the render tree, each
    /// owned by its picker node. This is only a registry for "dismiss every menu"
    /// (outside click) — it is Rc-pruned, never flush-order-indexed, so a dropped
    /// picker's handle falls out by strong count and the live ones are deduplicated.
    pub(crate) node_picker_menus: Vec<Rc<Cell<bool>>>,
    /// The anchored overlays this frame's flush registered — each anchor's live
    /// hit-space bounds, placement and handles. The post-flush
    /// `render_anchored_overlays` drains it.
    pub(crate) anchored_overlays: Vec<RegisteredAnchoredOverlay>,
    /// The anchored overlays drawn this frame — bounds, dismissal mode and
    /// binding — for the pointer-down outside-interaction dismissal and the
    /// anchor-left-the-tree close.
    pub(crate) presented_anchored_overlays: Vec<PresentedAnchoredOverlay>,
}

#[derive(Clone)]
pub struct PickerMenuEntry {
    pub label: String,
    pub(crate) tag: Id,
}

pub struct PickerMenuRequest {
    pub entries: Vec<PickerMenuEntry>,
    pub(crate) selection: Binding<Id>,
    pub(crate) open: Rc<Cell<bool>>,
    pub(crate) origin: LayoutPoint,
    pub(crate) width: f64,
    pub(crate) row_height: f64,
    pub(crate) selected: Id,
}

impl PopupMenuState {
    /// Register a picker node's open handle for "dismiss every menu". Prunes handles
    /// of dropped pickers (the registry is then their only holder) and deduplicates,
    /// so a node re-registering its handle on every flush is idempotent.
    pub(crate) fn register_picker_menu(&mut self, open: &Rc<Cell<bool>>) {
        self.node_picker_menus
            .retain(|handle| Rc::strong_count(handle) > 1);
        if !self
            .node_picker_menus
            .iter()
            .any(|handle| Rc::ptr_eq(handle, open))
        {
            self.node_picker_menus.push(Rc::clone(open));
        }
    }

    /// Close every open picker menu (an outside click dismisses them all).
    pub(crate) fn close_all_picker_menus(&mut self) {
        self.node_picker_menus
            .retain(|handle| Rc::strong_count(handle) > 1);
        for handle in &self.node_picker_menus {
            handle.set(false);
        }
    }
}

impl PopupMenuStateGroup {
    pub(crate) fn new() -> Self {
        Self(Rc::new(RefCell::new(Vec::new())))
    }

    pub(crate) fn push(&self, state: Binding<WindowState>) {
        self.0.borrow_mut().push(state);
    }

    pub(crate) fn truncate(&self, len: usize) {
        let mut states = self.0.borrow_mut();
        for state in states.drain(len..) {
            state.set(WindowState::Closed);
        }
    }

    pub(crate) fn close_all(&self) {
        self.truncate(0);
    }
}

impl_extractor!(PopupMenuStateGroup);

pub fn popup_window_origin(origin: LayoutPoint, env: &Environment) -> LayoutPoint {
    let window_origin = env
        .get::<HydrolysisWindowOrigin>()
        .copied()
        .expect("hydrolysis popup windows require HydrolysisWindowOrigin in environment");
    LayoutPoint::new(window_origin.x + origin.x, window_origin.y + origin.y)
}

const fn popup_enter_animation() -> Animation {
    Animation::bezier(Duration::from_millis(120), 0.2, 0.0, 0.0, 1.0)
}

fn animated_popup_panel(content: impl View, group: PopupMenuStateGroup) -> impl View {
    let opacity = Binding::f32(0.0);
    let scale = Binding::f32(0.96);
    let enter_animation = popup_enter_animation();
    content
        .opacity(opacity.with(enter_animation.clone()))
        .scale(
            scale.with(enter_animation.clone()),
            scale.with(enter_animation),
        )
        .on_appear(move || {
            opacity.set(1.0);
            scale.set(1.0);
        })
        .with(group)
}

/// Marker on the wrapped menu rows: a `PopupWindowManager` menu's panel is
/// the theme's drawn context-menu surface — container colour, corner shape
/// and elevation — rather than a view-level fill, so it renders identically
/// to the drawn `.context_menu` presentation in dark and light
/// (water-rs/hydrolysis#200). The window leaves `POPUP_MENU_PANEL_MARGIN` of
/// transparent room on every side for the panel's elevation shadow.
pub struct PopupMenuSurface;
impl MetadataKey for PopupMenuSurface {}

/// Transparent margin a `PopupWindowManager` menu window leaves around its
/// panel, in logical points — the room the panel's elevation shadow draws
/// into inside the window's own surface.
pub const POPUP_MENU_PANEL_MARGIN: f64 = 14.0;

/// A divider row's height: the theme's separator line
/// (`md.comp.menu.divider.height`, 1 dp) inside the menu's vertical padding
/// (`md.comp.menu.container.top-space`/`bottom-space`, 8 dp each side).
pub const fn popup_menu_divider_height(metrics: TextContextMenuMetrics) -> f64 {
    metrics
        .vertical_padding
        .mul_add(2.0, metrics.separator_thickness)
}

/// The horizontal gap between a row's label and its shortcut hint.
const MENU_SHORTCUT_HINT_GAP: f64 = 12.0;

/// The text measurements [`popup_menu_size`] consumes: every row's intrinsic
/// label/supporting-line width (the widest wins), the supporting line's
/// intrinsic height, and the row's horizontal label inset — the theme's
/// menu-item inset, which every row's leading edge shares.
#[derive(Clone, Copy)]
pub struct PopupMenuTextMetrics {
    /// The height a supporting (caption) line adds to a subtitled row.
    pub(crate) subtitle_height: f64,
    /// The widest label or supporting line across the menu, measured
    /// intrinsically — a row never wraps mid-word into a clipped column.
    pub(crate) max_row_text_width: f64,
    /// The widest shortcut hint across the menu, `0.0` when no command
    /// carries one — measured in the label-large supporting style.
    pub(crate) max_hint_width: f64,
    /// The horizontal inset between a row's edge and its label —
    /// `md.comp.menu.list-item.leading-space`/`trailing-space` (12 dp in M3).
    pub row_inset: f64,
}

pub fn popup_menu_size(
    nodes: &[PopupMenuNode],
    metrics: TextContextMenuMetrics,
    text: &PopupMenuTextMetrics,
) -> (f64, f64) {
    // `md.comp.menu.container.min-width`/`max-width` (112/280 dp).
    // Shortcut hints widen the menu so a row never overlaps its label.
    let hint_width = if text.max_hint_width > 0.0 {
        text.max_hint_width + MENU_SHORTCUT_HINT_GAP
    } else {
        0.0
    };
    let width = (text.row_inset.mul_add(2.0, text.max_row_text_width) + hint_width)
        .clamp(metrics.min_width, metrics.max_width);
    // `md.comp.menu.list-item.container.height` (48 dp) per row — a
    // subtitled row grows by its supporting line — plus the container's
    // `top-space`/`bottom-space`.
    let height = metrics.vertical_padding.mul_add(
        2.0,
        nodes
            .iter()
            .map(|node| match node {
                PopupMenuNode::Command {
                    subtitle: Some(_), ..
                } => metrics.row_height + text.subtitle_height,
                PopupMenuNode::Divider => popup_menu_divider_height(metrics),
                _ => metrics.row_height,
            })
            .sum::<f64>()
            .max(metrics.row_height),
    );
    (width, height)
}

/// A menu command row: a borderless `Button` that closes the menu group and
/// runs the command on press. A `disabled` row draws its label at Material's
/// disabled contrast — on-surface at 38% opacity, which here is the menu's
/// `Foreground` token at 0.38 — while its press stays inert (the action's
/// early return also gates it).
fn popup_menu_command_row(
    label: SemanticLabel,
    action: SharedAction<()>,
    disabled: bool,
) -> AnyView {
    let button = Button::new(label).style(ButtonStyle::Borderless).action(
        move |group: PopupMenuStateGroup, env: Environment| {
            if disabled {
                return;
            }
            group.close_all();
            call_action_discarding_result(&action, &env);
        },
    );
    if disabled {
        AnyView::new(
            button.foreground(Color::new(waterui::theme::color::Foreground).with_opacity(0.38)),
        )
    } else {
        AnyView::new(button)
    }
}

/// A row's trailing shortcut hint in the theme's menu-label treatment: the
/// borderless-button label font the row's own label resolves through
/// [`crate::engine::WidgetTheme::button_label_font`] — label-large in the
/// Material mapping — over the muted (on-surface-variant) colour, dimmed
/// with the row's label when the command is disabled. The drawn row and
/// [`SemanticCore::popup_menu_text_metrics`] share it so the two cannot
/// drift (water-rs/hydrolysis#247).
fn shortcut_hint_styled(
    shortcut: &Shortcut,
    disabled: bool,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> StyledStr {
    let mut styled = StyledStr::plain(shortcut_hint_text(shortcut)).foreground({
        let color = Color::new(waterui::theme::color::MutedForeground);
        if disabled {
            color.with_opacity(0.38)
        } else {
            color
        }
    });
    if let Some(font) = theme.button_label_font(ButtonStyle::Borderless) {
        styled = styled.font(font);
    }
    styled
}

/// The menu's row content shared by the popup-window and the drawn
/// `.context_menu` presentation: one row per node — borderless commands,
/// dividers and submenu items — padded by the theme's vertical padding. The
/// chrome around it (the borderless window's rounded `Surface` background,
/// the drawn presentation's theme panel) is the caller's.
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn popup_menu_content(
    nodes: Vec<PopupMenuNode>,
    depth: usize,
    metrics: TextContextMenuMetrics,
    text: PopupMenuTextMetrics,
    popup_origin: LayoutPoint,
    width: f64,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> AnyView {
    let mut rows = Vec::with_capacity(nodes.len());
    let mut row_top = metrics.vertical_padding;
    for node in nodes {
        let row_height = match &node {
            PopupMenuNode::Command {
                subtitle: Some(_), ..
            } => metrics.row_height + text.subtitle_height,
            PopupMenuNode::Divider => popup_menu_divider_height(metrics),
            _ => metrics.row_height,
        };
        match node {
            PopupMenuNode::Command {
                label,
                action,
                disabled,
                shortcut,
                ..
            } => {
                let disabled = disabled.snapshot();
                let button = popup_menu_command_row(label, action, disabled);
                // The chord's trailing hint is a sibling pinned to the row's
                // trailing inset by a spacer, styled by the shared builder
                // (water-rs/hydrolysis#247).
                let row_content = match shortcut {
                    Some(shortcut) => AnyView::new(
                        hstack((
                            AnyView::new(button),
                            spacer(),
                            waterui_text::text(shortcut_hint_styled(&shortcut, disabled, theme))
                                .padding_with(EdgeInsets::new(
                                    0.0,
                                    0.0,
                                    0.0,
                                    crate::num_cast::f64_as_f32(metrics.horizontal_padding),
                                )),
                        ))
                        .spacing(0.0),
                    ),
                    None => AnyView::new(button),
                };
                // A leading-aligned frame puts the content-width row at the
                // menu's leading edge, so every row's label shares one leading
                // x regardless of kind.
                rows.push(AnyView::new(
                    Frame::new(row_content)
                        .height(crate::num_cast::f64_as_f32(row_height))
                        .max_width(f32::INFINITY)
                        .alignment(waterui_layout::alignment::Leading),
                ));
            }
            PopupMenuNode::Divider => rows.push(AnyView::new(
                Frame::new(Divider)
                    .height(crate::num_cast::f64_as_f32(row_height))
                    .max_width(f32::INFINITY)
                    .alignment(waterui_layout::alignment::Leading),
            )),
            PopupMenuNode::Menu { label, items, .. } => {
                let next_depth = depth + 1;
                let child_origin = LayoutPoint::new(
                    popup_origin.x + crate::num_cast::f64_as_f32(width),
                    popup_origin.y + crate::num_cast::f64_as_f32(row_top),
                );
                let button = Button::new(label).style(ButtonStyle::Borderless).action({
                    let theme = theme.clone();
                    move |group: PopupMenuStateGroup, env: Environment| {
                        if items.is_empty() {
                            return;
                        }
                        group.truncate(next_depth);
                        let (window, child_state) = popup_menu_window(
                            items.clone(),
                            child_origin,
                            group.clone(),
                            next_depth,
                            metrics,
                            text,
                            &theme,
                        );
                        group.push(child_state);
                        env.get::<PopupWindowManager>()
                            .expect(
                                "hydrolysis popup menus require PopupWindowManager in environment",
                            )
                            .show(window, &env);
                    }
                });
                // The button sizes to its label: a leading-aligned frame puts
                // the content-width row at the menu's leading edge, so every
                // row's label shares one leading x regardless of kind.
                rows.push(AnyView::new(
                    Frame::new(button)
                        .height(crate::num_cast::f64_as_f32(row_height))
                        .max_width(f32::INFINITY)
                        .alignment(waterui_layout::alignment::Leading),
                ));
            }
        }
        row_top += row_height;
    }
    let menu_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> = rows.into_iter().collect();
    AnyView::new(
        menu_content
            .alignment(HorizontalAlignment::Leading)
            .spacing(0.0)
            .padding_with(EdgeInsets::symmetric(
                crate::num_cast::f64_as_f32(metrics.vertical_padding),
                0.0,
            ))
            // A menu row's label is body text that happens to be tappable,
            // not button chrome: it draws in the foreground colour, with the
            // destructive role's explicit error colour still winning.
            .with(ListRowChrome)
            // A menu row is not a button: its inset is the theme's menu-item
            // inset (`md.comp.menu.list-item.leading-space`/`trailing-space`,
            // 12 dp in M3), its state layer is the on-surface colour under
            // `md.sys.state.*` opacities, and the label colours stay semantic
            // (`label_color: None`).
            .install(InteractionStyle::new(
                ButtonMetrics::new(metrics.horizontal_padding, 0.0, 0.0, 0.0),
                Color::new(waterui::theme::color::Foreground),
                kurbo::RoundedRectRadii::from(0.0),
            )),
    )
}

pub fn popup_menu_window(
    nodes: Vec<PopupMenuNode>,
    origin: LayoutPoint,
    group: PopupMenuStateGroup,
    depth: usize,
    metrics: TextContextMenuMetrics,
    text: PopupMenuTextMetrics,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let (width, height) = popup_menu_size(&nodes, metrics, &text);
    let group_for_content = group;
    let state_for_content = state.clone();
    let nodes_for_content = nodes;
    let theme = theme.clone();
    let popup_content = move || {
        AnyView::new(animated_popup_panel(
            Metadata::new(
                popup_menu_content(
                    nodes_for_content.clone(),
                    depth,
                    metrics,
                    text,
                    origin,
                    width,
                    &theme,
                ),
                PopupMenuSurface,
            )
            .padding_with(crate::num_cast::f64_as_f32(POPUP_MENU_PANEL_MARGIN)),
            group_for_content.clone(),
        ))
    };
    let mut popup = Window::new(
        TEXT_CONTEXT_MENU_WINDOW_TITLE,
        state_for_content,
        popup_content,
    )
    .style(WindowStyle::Borderless)
    .resizable(false)
    .background(Color::transparent());
    popup.closable = false;
    // The window inflates by the panel margin on every side: the menu panel
    // draws inside the inset, and the transparent ring gives the panel's
    // elevation shadow room inside the window's own surface instead of
    // being clipped at the frame.
    popup.frame.set(LayoutRect::new(
        LayoutPoint::new(
            origin.x - crate::num_cast::f64_as_f32(POPUP_MENU_PANEL_MARGIN),
            origin.y - crate::num_cast::f64_as_f32(POPUP_MENU_PANEL_MARGIN),
        ),
        LayoutSize::new(
            crate::num_cast::f64_as_f32(POPUP_MENU_PANEL_MARGIN.mul_add(2.0, width)),
            crate::num_cast::f64_as_f32(POPUP_MENU_PANEL_MARGIN.mul_add(2.0, height)),
        ),
    ));
    (popup, state)
}

/// The semantic counterpart of [`popup_menu_window`]: the same rows —
/// borderless `Button` commands, dividers, and submenu items that open a
/// deeper window — without the metrics-dependent chrome. A semantic window
/// has no frame to size and no corner chrome to round, so none is built; the
/// emitted accessibility tree is identical to the rendered popup's.
#[cfg(feature = "accessibility")]
pub fn semantic_popup_menu_window(
    nodes: Vec<PopupMenuNode>,
    group: PopupMenuStateGroup,
    depth: usize,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let group_for_content = group;
    let state_for_content = state.clone();
    let nodes_for_content = nodes;
    let popup_content = move || {
        let mut rows = Vec::with_capacity(nodes_for_content.len());
        for node in nodes_for_content.clone() {
            match node {
                PopupMenuNode::Command {
                    label,
                    action,
                    disabled,
                    ..
                } => {
                    let button = popup_menu_command_row(label, action, disabled.snapshot());
                    rows.push(AnyView::new(button));
                }
                PopupMenuNode::Divider => rows.push(AnyView::new(Divider)),
                PopupMenuNode::Menu { label, items, .. } => {
                    let next_depth = depth + 1;
                    let button = Button::new(label).style(ButtonStyle::Borderless).action(
                        move |group: PopupMenuStateGroup, env: Environment| {
                            if items.is_empty() {
                                return;
                            }
                            group.truncate(next_depth);
                            let (window, child_state) =
                                semantic_popup_menu_window(items.clone(), group.clone(), next_depth);
                            group.push(child_state);
                            env.get::<PopupWindowManager>()
                                .expect(
                                    "hydrolysis popup menus require PopupWindowManager in environment",
                                )
                                .show(window, &env);
                        },
                    );
                    rows.push(AnyView::new(button));
                }
            }
        }
        let menu_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
            rows.into_iter().collect();
        AnyView::new(
            menu_content
                .alignment(HorizontalAlignment::Leading)
                .spacing(0.0)
                .with(group_for_content.clone()),
        )
    };
    let mut popup = Window::new(
        TEXT_CONTEXT_MENU_WINDOW_TITLE,
        state_for_content,
        popup_content,
    )
    .style(WindowStyle::Borderless)
    .resizable(false)
    .background(Color::transparent());
    popup.closable = false;
    (popup, state)
}

/// The semantic counterpart of [`picker_menu_window`]: the same selection
/// rows — borderless `Button`s that write the binding and close the menu —
/// without the frame sizing or panel chrome.
#[cfg(feature = "accessibility")]
pub fn semantic_picker_menu_window(
    entries: Vec<PickerMenuEntry>,
    selection: Binding<Id>,
    open: Rc<Cell<bool>>,
    group: PopupMenuStateGroup,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let group_for_content = group;
    let state_for_content = state.clone();
    let entries_for_content = entries;
    let popup_content = move || {
        let mut rows = Vec::with_capacity(entries_for_content.len());
        for entry in entries_for_content.clone() {
            let label = entry.label.clone();
            let target = entry.tag;
            let row_selection = selection.clone();
            let row_group = group_for_content.clone();
            let row_open = Rc::clone(&open);
            rows.push(AnyView::new(
                button(label)
                    .style(ButtonStyle::Borderless)
                    .action(move || {
                        if row_selection.snapshot() != target {
                            row_selection.set(target);
                        }
                        row_open.set(false);
                        row_group.close_all();
                    }),
            ));
        }
        let menu_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
            rows.into_iter().collect();
        AnyView::new(
            menu_content
                .alignment(HorizontalAlignment::Leading)
                .spacing(0.0)
                .with(group_for_content.clone()),
        )
    };
    let mut popup = Window::new("WaterUI Picker Menu", state_for_content, popup_content)
        .style(WindowStyle::Borderless)
        .resizable(false)
        .background(Color::transparent());
    popup.closable = false;
    (popup, state)
}

#[allow(
    clippy::too_many_arguments,
    reason = "fully specifies a popup-menu window layout; grouping into a struct would not improve clarity"
)]
pub fn picker_menu_window(
    entries: Vec<PickerMenuEntry>,
    selection: Binding<Id>,
    open: Rc<Cell<bool>>,
    origin: LayoutPoint,
    width: f64,
    row_height: f64,
    selected: Id,
    group: PopupMenuStateGroup,
    metrics: PickerMetrics,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let height = row_height * crate::num_cast::usize_as_f64(entries.len());
    let state_for_content = state.clone();
    let group_for_content = group;
    let entries_for_content = entries;
    let popup_content =
        move || {
            let mut rows = Vec::with_capacity(entries_for_content.len());
            for entry in entries_for_content.clone() {
                let label = entry.label.clone();
                let target = entry.tag;
                let row_selection = selection.clone();
                let row_group = group_for_content.clone();
                let row_open = Rc::clone(&open);
                let row = Frame::new(button(label).style(ButtonStyle::Borderless).action(
                    move || {
                        if row_selection.snapshot() != target {
                            row_selection.set(target);
                        }
                        row_open.set(false);
                        row_group.close_all();
                    },
                ))
                .width(crate::num_cast::f64_as_f32(width))
                .height(crate::num_cast::f64_as_f32(row_height));
                if target == selected {
                    rows.push(AnyView::new(row.background(
                        RoundedRectangle::new(0.0).fill(Color::new(Surface).with_opacity(0.84)),
                    )));
                } else {
                    rows.push(AnyView::new(row));
                }
            }
            let menu_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
                rows.into_iter().collect();
            let panel = menu_content
                .alignment(HorizontalAlignment::Leading)
                .spacing(0.0)
                .background(
                    FixedRoundedRectangle::new(crate::num_cast::f64_as_f32(
                        metrics.popup_corner_radius,
                    ))
                    .fill(Color::new(Surface).with_opacity(0.96)),
                );
            AnyView::new(animated_popup_panel(panel, group_for_content.clone()))
        };
    let mut popup = Window::new("WaterUI Picker Menu", state_for_content, popup_content)
        .style(WindowStyle::Borderless)
        .resizable(false)
        .background(Color::transparent());
    popup.closable = false;
    popup.frame.set(LayoutRect::new(
        origin,
        LayoutSize::new(
            crate::num_cast::f64_as_f32(width),
            crate::num_cast::f64_as_f32(height),
        ),
    ));
    (popup, state)
}

fn color_picker_palette() -> [(&'static str, Color); 12] {
    [
        ("Red", Color::srgb(0xba, 0x1a, 0x1a)),
        ("Orange", Color::srgb(0xc2, 0x41, 0x0c)),
        ("Amber", Color::srgb(0x8a, 0x5a, 0x00)),
        ("Green", Color::srgb(0x2e, 0x7d, 0x32)),
        ("Teal", Color::srgb(0x00, 0x79, 0x6b)),
        ("Cyan", Color::srgb(0x00, 0x6d, 0x8f)),
        ("Blue", Color::srgb(0x0b, 0x57, 0xd0)),
        ("Indigo", Color::srgb(0x44, 0x38, 0xca)),
        ("Purple", Color::srgb(0x7b, 0x1f, 0xa2)),
        ("Pink", Color::srgb(0xa0, 0x18, 0x55)),
        ("Brown", Color::srgb(0x79, 0x55, 0x48)),
        ("Grey", Color::srgb(0x5f, 0x63, 0x68)),
    ]
}

/// The color-picker panel's rendered extent — swatch grid plus the optional
/// alpha/headroom rows. Placement is a rendered-frame concern; the semantic
/// window carries no origin.
fn color_picker_size(support_alpha: bool, support_hdr: bool) -> (f64, f64) {
    let width = 280.0;
    let swatch = 40.0;
    let gap = 8.0;
    let rows = 3.0;
    let alpha_row_height = if support_alpha { 48.0 } else { 0.0 };
    let hdr_row_height = if support_hdr { 48.0 } else { 0.0 };
    let height = 2.0f64.mul_add(gap, f64::mul_add(rows, swatch, 16.0))
        + alpha_row_height
        + hdr_row_height
        + 16.0;
    (width, height)
}

/// The color-picker window itself: palette swatches plus the optional
/// alpha/headroom rows. Shared by the rendered popup path (which then sets a
/// frame) and the semantic activation path (which mounts the window with no
/// placement at all).
fn color_picker_window_base(
    value: Binding<Color>,
    support_alpha: bool,
    support_hdr: bool,
    group: PopupMenuStateGroup,
    env: &Environment,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let (width, _) = color_picker_size(support_alpha, support_hdr);
    let swatch = 40.0;
    let gap = 8.0;
    let group_for_content = group;
    let state_for_content = state.clone();
    let popup_env = env.clone();
    let popup_content = move || {
        let palette = color_picker_palette();
        let mut row_views = Vec::with_capacity(5);
        for row_index in 0..3 {
            let mut swatches = Vec::with_capacity(4);
            for column_index in 0..4 {
                let palette_index = row_index * 4 + column_index;
                let (label_key, color) = palette[palette_index].clone();
                let label = crate::localization::text(&popup_env, label_key);
                let selected = value.clone();
                let group = group_for_content.clone();
                let swatch_color = color.clone();
                swatches.push(AnyView::new(
                    Frame::new(
                        button(label)
                            .style(ButtonStyle::Borderless)
                            .action(move || {
                                selected.set(color.clone());
                                group.close_all();
                            })
                            .install(LabelDisplayMode::Hidden)
                            .background(RoundedRectangle::new(0.2).fill(swatch_color)),
                    )
                    .width(crate::num_cast::f64_as_f32(swatch))
                    .height(crate::num_cast::f64_as_f32(swatch)),
                ));
            }
            let row: waterui_layout::stack::HStack<(Vec<AnyView>,)> =
                swatches.into_iter().collect();
            row_views.push(AnyView::new(row.spacing(gap)));
        }

        if support_alpha {
            let selected = value.clone();
            let group = group_for_content.clone();
            row_views.push(AnyView::new(
                Frame::new(
                    button(crate::localization::text(&popup_env, "opacity_50"))
                        .style(ButtonStyle::Borderless)
                        .action(move || {
                            let current = selected.snapshot();
                            selected.set(current.with_opacity(0.5));
                            group.close_all();
                        }),
                )
                .width(crate::num_cast::f64_as_f32(width - 32.0))
                .height(40.0),
            ));
        }

        if support_hdr {
            let selected = value.clone();
            let group = group_for_content.clone();
            row_views.push(AnyView::new(
                Frame::new(
                    button(crate::localization::text(&popup_env, "hdr_headroom"))
                        .style(ButtonStyle::Borderless)
                        .action(move || {
                            let current = selected.snapshot();
                            selected.set(current.with_headroom(1.0));
                            group.close_all();
                        }),
                )
                .width(crate::num_cast::f64_as_f32(width - 32.0))
                .height(40.0),
            ));
        }

        let content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
            row_views.into_iter().collect();
        let panel = content
            .alignment(HorizontalAlignment::Leading)
            .spacing(gap)
            .padding_with(EdgeInsets::all(16.0))
            .background(RoundedRectangle::new(0.05).fill(Color::new(Surface).with_opacity(0.96)));
        AnyView::new(animated_popup_panel(panel, group_for_content.clone()))
    };
    let mut popup = Window::new("WaterUI Color Picker", state_for_content, popup_content)
        .style(WindowStyle::Borderless)
        .resizable(false)
        .background(Color::transparent());
    popup.closable = false;
    (popup, state)
}

/// The rendered popup path: the shared color-picker window anchored at the
/// trigger's resolved origin. The semantic activation path mounts
/// [`color_picker_window_base`] directly and sets no frame.
pub fn color_picker_window(
    value: Binding<Color>,
    support_alpha: bool,
    support_hdr: bool,
    origin: LayoutPoint,
    group: PopupMenuStateGroup,
    env: &Environment,
) -> (Window, Binding<WindowState>) {
    let (popup, state) = color_picker_window_base(value, support_alpha, support_hdr, group, env);
    let (width, height) = color_picker_size(support_alpha, support_hdr);
    popup.frame.set(LayoutRect::new(
        origin,
        LayoutSize::new(
            crate::num_cast::f64_as_f32(width),
            crate::num_cast::f64_as_f32(height),
        ),
    ));
    (popup, state)
}

fn time_part_binding(value: &Binding<i32>, range: RangeInclusive<i32>, label: String) -> Stepper {
    stepper(label, value)
        .range(range)
        .value_formatter(|part| format!("{part:02}"))
}

fn apply_staged_date_time(
    value: &Binding<DateTime>,
    range: &RangeInclusive<DateTime>,
    date: Date,
    hour: i32,
    minute: i32,
    second: i32,
) {
    let hour = i8::try_from(hour).expect("date picker staged hour must fit i8");
    let minute = i8::try_from(minute).expect("date picker staged minute must fit i8");
    let second = i8::try_from(second).expect("date picker staged second must fit i8");
    let current = value.snapshot();
    let time = current.time();
    let next = date
        .at(hour, minute, second, time.subsec_nanosecond())
        .clamp(*range.start(), *range.end());
    value.set(next);
}

/// The date-picker panel's rendered extent from the picker's field
/// configuration — date calendar plus optional time rows. Placement is a
/// rendered-frame concern; the semantic window carries no origin.
const fn date_picker_size(ty: DatePickerType) -> (f64, f64) {
    let uses_date = matches!(
        ty,
        DatePickerType::Date
            | DatePickerType::DateHourAndMinute
            | DatePickerType::DateHourMinuteAndSecond
    );
    let uses_time = !matches!(ty, DatePickerType::Date);
    let width = 360.0;
    let height = if uses_date && uses_time {
        520.0
    } else if uses_date {
        430.0
    } else {
        260.0
    };
    (width, height)
}

/// The date-picker window itself: calendar and time-stepper staging rows plus
/// the cancel/apply bar. Shared by the rendered popup path (which then sets a
/// frame) and the semantic activation path (which mounts the window with no
/// placement at all).
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
fn date_picker_window_base(
    value: Binding<DateTime>,
    range: RangeInclusive<DateTime>,
    ty: DatePickerType,
    group: PopupMenuStateGroup,
    env: &Environment,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let current = value.snapshot().clamp(*range.start(), *range.end());
    let staged_date = Binding::container(current.date());
    let staged_visible_month = Binding::container(current.date());
    let current_time = current.time();
    let staged_hour = Binding::container(i32::from(current_time.hour()));
    let staged_minute = Binding::container(i32::from(current_time.minute()));
    let staged_second = Binding::container(i32::from(current_time.second()));
    let uses_date = matches!(
        ty,
        DatePickerType::Date
            | DatePickerType::DateHourAndMinute
            | DatePickerType::DateHourMinuteAndSecond
    );
    let uses_time = !matches!(ty, DatePickerType::Date);
    let uses_second = matches!(
        ty,
        DatePickerType::HourMinuteAndSecond | DatePickerType::DateHourMinuteAndSecond
    );
    let range_start = *range.start();
    let range_end = *range.end();
    let group_for_content = group;
    let state_for_content = state.clone();
    let popup_env = env.clone();
    let popup_content = move || {
        let mut sections = Vec::new();
        sections.push(AnyView::new(
            text(crate::localization::text(&popup_env, "select_date")).headline(),
        ));
        if uses_date {
            let visible_month = staged_visible_month.clone();
            sections.push(AnyView::new(
                Calendar::new(
                    crate::localization::text(&popup_env, "date"),
                    &staged_date,
                    &visible_month,
                )
                .range(range_start.date()..=range_end.date())
                .hide_label(),
            ));
        }
        if uses_time {
            let mut time_controls = Vec::new();
            time_controls.push(AnyView::new(time_part_binding(
                &staged_hour,
                0..=23,
                crate::localization::text(&popup_env, "hour"),
            )));
            time_controls.push(AnyView::new(time_part_binding(
                &staged_minute,
                0..=59,
                crate::localization::text(&popup_env, "minute"),
            )));
            if uses_second {
                time_controls.push(AnyView::new(time_part_binding(
                    &staged_second,
                    0..=59,
                    crate::localization::text(&popup_env, "second"),
                )));
            }
            let time_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
                time_controls.into_iter().collect();
            sections.push(AnyView::new(time_content.spacing(8.0)));
        }
        let cancel_group = group_for_content.clone();
        let apply_group = group_for_content.clone();
        let apply_value = value.clone();
        let apply_range = range.clone();
        let apply_date = staged_date.clone();
        let apply_hour = staged_hour.clone();
        let apply_minute = staged_minute.clone();
        let apply_second = staged_second.clone();
        sections.push(AnyView::new(
            hstack((
                spacer(),
                button(crate::localization::text(&popup_env, "cancel"))
                    .style(ButtonStyle::Borderless)
                    .action(move || {
                        cancel_group.close_all();
                    }),
                button(crate::localization::text(&popup_env, "ok"))
                    .style(ButtonStyle::Borderless)
                    .action(move || {
                        apply_staged_date_time(
                            &apply_value,
                            &apply_range,
                            apply_date.snapshot(),
                            apply_hour.snapshot(),
                            apply_minute.snapshot(),
                            if uses_second {
                                apply_second.snapshot()
                            } else {
                                0
                            },
                        );
                        apply_group.close_all();
                    }),
            ))
            .spacing(8.0),
        ));

        let content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
            sections.into_iter().collect();
        let panel = content
            .alignment(HorizontalAlignment::Leading)
            .spacing(16.0)
            .padding_with(EdgeInsets::all(24.0))
            .background(RoundedRectangle::new(0.04).fill(Color::new(Surface).with_opacity(0.96)));
        AnyView::new(animated_popup_panel(panel, group_for_content.clone()))
    };
    let mut popup = Window::new("WaterUI Date Picker", state_for_content, popup_content)
        .style(WindowStyle::Borderless)
        .resizable(false)
        .background(Color::transparent());
    popup.closable = false;
    (popup, state)
}

/// The rendered popup path: the shared date-picker window anchored at the
/// trigger's resolved origin. The semantic activation path mounts
/// [`date_picker_window_base`] directly and sets no frame.
pub fn date_picker_window(
    value: Binding<DateTime>,
    range: RangeInclusive<DateTime>,
    ty: DatePickerType,
    origin: LayoutPoint,
    group: PopupMenuStateGroup,
    env: &Environment,
) -> (Window, Binding<WindowState>) {
    let (popup, state) = date_picker_window_base(value, range, ty, group, env);
    let (width, height) = date_picker_size(ty);
    popup.frame.set(LayoutRect::new(
        origin,
        LayoutSize::new(
            crate::num_cast::f64_as_f32(width),
            crate::num_cast::f64_as_f32(height),
        ),
    ));
    (popup, state)
}

impl SemanticCore {
    pub(crate) const fn active_popup_menu_visible(&self) -> bool {
        self.popup_menu.active_popup_menu_group.is_some()
    }

    pub(crate) fn dismiss_active_popup_menu(&mut self) {
        // Dropping the presentation hands the preview and accessory back to
        // their owning node's slots, so a later open mounts them again.
        self.popup_menu.context_menu_presentation = None;
        if let Some(group) = self.popup_menu.active_popup_menu_group.take() {
            group.close_all();
        }
        self.popup_menu.close_all_picker_menus();
    }

    pub(crate) fn register_picker_menu(&mut self, open: &Rc<Cell<bool>>) {
        self.popup_menu.register_picker_menu(open);
    }

    /// Appends the debug build's "inspect this element" entry.
    ///
    /// A debug build offers it on a menu that already has items of its own,
    /// the way a browser's entry joins its own menus: it extends a menu, it
    /// never creates one — an empty `.context_menu` behaves the same in every
    /// build. A release build appends nothing.
    #[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
    pub(crate) fn append_inspect_element_item(
        &self,
        items: &mut Vec<PopupMenuNode>,
        point: kurbo::Point,
    ) {
        if !cfg!(debug_assertions) {
            return;
        }
        // Without a node there is nothing to reveal, and an entry that silently
        // does nothing is worse than no entry at all.
        let Some(node) = self.accessibility.node_at_point(point) else {
            return;
        };
        if !items.is_empty() {
            items.push(PopupMenuNode::Divider);
        }
        let plain_label = String::from("Inspect element");
        items.push(PopupMenuNode::Command {
            label: waterui_controls::label::label(plain_label.clone()),
            plain_label,
            action: waterui_core::handler::SharedAction::new(
                move |env: waterui_core::Environment| {
                    let Some(inspector) = env.get::<waterui::inspector::InspectorRuntime>() else {
                        return;
                    };
                    inspector.inspect_node(waterui_inspector_protocol::NodeId(node.0));
                },
            ),
            disabled: nami::Computed::constant(false),
            shortcut: None,
            subtitle: None,
        });
    }

    /// Inspecting an element means naming it in the accessibility tree, so a
    /// build without that tree has no name to send — and a browser page has no
    /// inspector endpoint to send it to.
    #[cfg(any(not(feature = "accessibility"), target_arch = "wasm32"))]
    // always an empty stub where it compiles — const would lie about the real variant
    #[allow(clippy::missing_const_for_fn)]
    pub(crate) fn append_inspect_element_item(
        &self,
        _items: &mut Vec<PopupMenuNode>,
        _point: kurbo::Point,
    ) {
        // The signature keeps `&self` so the call sites do not branch on the
        // build shape: the real variant needs the tree this menu belongs to.
        let _ = self;
    }

    /// The topmost context-menu target that covers `point` and wholly
    /// contains `rect`. A `.context_menu` attached to the surface at the point
    /// — or to any ancestor covering it — registers bounds that enclose the
    /// surface's window rect (a directly wrapping menu's bounds *are* the
    /// surface's, both computed as `hit_transform * layout bounds`). A menu
    /// whose bounds only overlap the surface belongs to a sibling and does not
    /// claim its secondary press.
    pub(crate) fn topmost_context_menu_target_enclosing(
        &self,
        point: kurbo::Point,
        rect: kurbo::Rect,
    ) -> Option<ContextMenuTarget> {
        self.hit_test
            .context_menu_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| {
                target.bounds.contains(point)
                    && target.bounds.x0 <= rect.x0
                    && target.bounds.y0 <= rect.y0
                    && target.bounds.x1 >= rect.x1
                    && target.bounds.y1 >= rect.y1
            })
            .max_by(|(left_index, left), (right_index, right)| {
                Self::target_hit_priority(left.depth, left.order, *left_index).cmp(
                    &Self::target_hit_priority(right.depth, right.order, *right_index),
                )
            })
            .map(|(_, target)| target.clone())
    }

    pub(crate) fn topmost_context_menu_target_at_point(
        &self,
        point: kurbo::Point,
    ) -> Option<ContextMenuTarget> {
        self.hit_test
            .context_menu_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .max_by(|(left_index, left), (right_index, right)| {
                Self::target_hit_priority(left.depth, left.order, *left_index).cmp(
                    &Self::target_hit_priority(right.depth, right.order, *right_index),
                )
            })
            .map(|(_, target)| target.clone())
    }

    /// The measured height of a command's secondary line, drawn in the theme's
    /// supporting text style. Measured once per presented menu — every
    /// subtitled row grows by the same line height.
    pub(crate) fn popup_menu_subtitle_height(&mut self, env: &Environment) -> f64 {
        let styled = StyledStr::plain("Ag").font(waterui_text::font::Caption);
        f64::from(
            HydrolysisRenderer::measure_text_intrinsic_size(&mut self.state, styled, env).height,
        )
    }

    /// The text measurements a popup menu's size comes from: every row's
    /// intrinsic label/supporting-line width — the widest wins, so a subtitle
    /// never wraps mid-word into a clipped column — the supporting line's
    /// intrinsic height, and the row's horizontal label inset, the theme's
    /// menu-item inset every row's leading edge shares
    /// (`md.comp.menu.list-item.leading-space`/`trailing-space`).
    pub(crate) fn popup_menu_text_metrics(
        &mut self,
        nodes: &[PopupMenuNode],
        metrics: TextContextMenuMetrics,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> PopupMenuTextMetrics {
        let row_inset = metrics.horizontal_padding;
        let mut max_row_text_width = 0.0_f64;
        let mut max_hint_width = 0.0_f64;
        for node in nodes {
            let mut consider = |state: &mut HydroState, styled: StyledStr| {
                let size = HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env);
                max_row_text_width = max_row_text_width.max(f64::from(size.width));
            };
            match node {
                PopupMenuNode::Command {
                    plain_label,
                    subtitle,
                    shortcut,
                    disabled,
                    ..
                } => {
                    consider(&mut self.state, StyledStr::plain(plain_label.clone()));
                    if let Some(subtitle) = subtitle {
                        consider(
                            &mut self.state,
                            StyledStr::plain(subtitle.clone()).font(waterui_text::font::Caption),
                        );
                    }
                    if let Some(shortcut) = shortcut {
                        // Colour does not change intrinsic width, so the
                        // enabled treatment measures the hint's true extent.
                        let size = HydrolysisRenderer::measure_text_intrinsic_size(
                            &mut self.state,
                            shortcut_hint_styled(shortcut, disabled.snapshot(), theme),
                            env,
                        );
                        max_hint_width = max_hint_width.max(f64::from(size.width));
                    }
                }
                PopupMenuNode::Menu { plain_label, .. } => {
                    consider(&mut self.state, StyledStr::plain(plain_label.clone()));
                }
                PopupMenuNode::Divider => {}
            }
        }
        PopupMenuTextMetrics {
            subtitle_height: self.popup_menu_subtitle_height(env),
            max_row_text_width,
            max_hint_width,
            row_inset,
        }
    }

    pub(crate) fn show_popup_menu_nodes(
        &mut self,
        nodes: Vec<PopupMenuNode>,
        origin: LayoutPoint,
        metrics: TextContextMenuMetrics,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> bool {
        if nodes.is_empty() {
            return false;
        }
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        let popup_origin = popup_window_origin(origin, env);
        let text = self.popup_menu_text_metrics(&nodes, metrics, env, theme);
        let (window, state) =
            popup_menu_window(nodes, popup_origin, group.clone(), 0, metrics, text, theme);
        group.push(state);
        env.get::<PopupWindowManager>()
            .expect("hydrolysis popup menus require PopupWindowManager in environment")
            .show(window, env);
        self.popup_menu.active_popup_menu_group = Some(group);
        true
    }

    /// The semantic half of showing a popup menu: the same window the rendered
    /// runtime opens, minus the metrics-dependent chrome. It mounts through
    /// `PopupWindowManager` as a semantic window, so the items appear in the
    /// merged accessibility tree exactly as they do in the rendered runtime —
    /// the semantic runtime has no pointer to open it and no frame to place.
    #[cfg(feature = "accessibility")]
    pub(crate) fn activate_popup_menu_nodes(
        &mut self,
        nodes: Vec<PopupMenuNode>,
        env: &Environment,
    ) -> bool {
        if nodes.is_empty() {
            return false;
        }
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        let (window, state) = semantic_popup_menu_window(nodes, group.clone(), 0);
        group.push(state);
        env.get::<PopupWindowManager>()
            .expect("hydrolysis popup menus require PopupWindowManager in environment")
            .show(window, env);
        self.popup_menu.active_popup_menu_group = Some(group);
        self.context_mark_layout();
        true
    }

    /// The semantic half of showing a picker menu: the same selection window
    /// the rendered runtime opens, without frame metrics.
    #[cfg(feature = "accessibility")]
    pub(crate) fn activate_picker_menu(
        &mut self,
        entries: Vec<PickerMenuEntry>,
        selection: Binding<Id>,
        open: &Rc<Cell<bool>>,
        env: &Environment,
    ) -> bool {
        if entries.is_empty() {
            return false;
        }
        self.dismiss_active_popup_menu();
        open.set(true);
        let group = PopupMenuStateGroup::new();
        let (window, state) =
            semantic_picker_menu_window(entries, selection, Rc::clone(open), group.clone());
        group.push(state);
        env.get::<PopupWindowManager>()
            .expect("hydrolysis picker menus require PopupWindowManager in environment")
            .show(window, env);
        self.popup_menu.active_popup_menu_group = Some(group);
        self.context_mark_layout();
        true
    }

    pub(crate) fn show_picker_menu(
        &mut self,
        request: PickerMenuRequest,
        metrics: PickerMetrics,
        env: &Environment,
    ) -> bool {
        if request.entries.is_empty() {
            return false;
        }
        self.dismiss_active_popup_menu();
        request.open.set(true);
        let group = PopupMenuStateGroup::new();
        let popup_origin = popup_window_origin(request.origin, env);
        let (window, state) = picker_menu_window(
            request.entries,
            request.selection,
            request.open,
            popup_origin,
            request.width,
            request.row_height,
            request.selected,
            group.clone(),
            metrics,
        );
        group.push(state);
        env.get::<PopupWindowManager>()
            .expect("hydrolysis picker menus require PopupWindowManager in environment")
            .show(window, env);
        self.popup_menu.active_popup_menu_group = Some(group);
        self.context_mark_layout();
        true
    }

    pub(crate) fn show_color_picker(
        &mut self,
        value: Binding<Color>,
        support_alpha: bool,
        support_hdr: bool,
        origin: LayoutPoint,
        env: &Environment,
    ) -> bool {
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        let popup_origin = popup_window_origin(origin, env);
        let (window, state) = color_picker_window(
            value,
            support_alpha,
            support_hdr,
            popup_origin,
            group.clone(),
            env,
        );
        group.push(state);
        env.get::<PopupWindowManager>()
            .expect("hydrolysis color picker requires PopupWindowManager in environment")
            .show(window, env);
        self.popup_menu.active_popup_menu_group = Some(group);
        true
    }

    pub(crate) fn show_date_picker(
        &mut self,
        value: Binding<DateTime>,
        range: RangeInclusive<DateTime>,
        ty: DatePickerType,
        origin: LayoutPoint,
        env: &Environment,
    ) -> bool {
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        let popup_origin = popup_window_origin(origin, env);
        let (window, state) =
            date_picker_window(value, range, ty, popup_origin, group.clone(), env);
        group.push(state);
        env.get::<PopupWindowManager>()
            .expect("hydrolysis date picker requires PopupWindowManager in environment")
            .show(window, env);
        self.popup_menu.active_popup_menu_group = Some(group);
        true
    }

    /// The semantic half of showing a color picker: the same panel window the
    /// rendered runtime opens, mounted with no placement — a semantic popup
    /// has no frame to anchor and no pointer origin to anchor it to.
    #[cfg(feature = "accessibility")]
    pub(crate) fn activate_color_picker(
        &mut self,
        value: Binding<Color>,
        support_alpha: bool,
        support_hdr: bool,
        env: &Environment,
    ) -> bool {
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        let (window, state) =
            color_picker_window_base(value, support_alpha, support_hdr, group.clone(), env);
        group.push(state);
        env.get::<PopupWindowManager>()
            .expect("hydrolysis color picker requires PopupWindowManager in environment")
            .show(window, env);
        self.popup_menu.active_popup_menu_group = Some(group);
        self.context_mark_layout();
        true
    }

    /// The semantic half of showing a date picker: the same staging window
    /// the rendered runtime opens, mounted with no placement.
    #[cfg(feature = "accessibility")]
    pub(crate) fn activate_date_picker(
        &mut self,
        value: Binding<DateTime>,
        range: RangeInclusive<DateTime>,
        ty: DatePickerType,
        env: &Environment,
    ) -> bool {
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        let (window, state) = date_picker_window_base(value, range, ty, group.clone(), env);
        group.push(state);
        env.get::<PopupWindowManager>()
            .expect("hydrolysis date picker requires PopupWindowManager in environment")
            .show(window, env);
        self.popup_menu.active_popup_menu_group = Some(group);
        self.context_mark_layout();
        true
    }

    pub(crate) fn register_context_menu_target(
        &mut self,
        bounds: kurbo::Rect,
        items: nami::Computed<Vec<ResolvedMenuItem>>,
        env: &Environment,
        dismiss_requests: nami::Computed<i32>,
        preview: Rc<RefCell<Option<RetainedSubview>>>,
        accessory: Rc<RefCell<Option<RetainedSubview>>>,
    ) {
        self.register_context_menu_target_data(
            bounds,
            items,
            self.render_depth,
            env,
            dismiss_requests,
            preview,
            accessory,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register_context_menu_target_data(
        &mut self,
        bounds: kurbo::Rect,
        items: nami::Computed<Vec<ResolvedMenuItem>>,
        depth: usize,
        env: &Environment,
        dismiss_requests: nami::Computed<i32>,
        preview: Rc<RefCell<Option<RetainedSubview>>>,
        accessory: Rc<RefCell<Option<RetainedSubview>>>,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.hit_test.next_hit_test_order();
        let bounds = self.hit_test.clip_hit_bounds(bounds);
        self.hit_test.context_menu_targets.push(ContextMenuTarget {
            bounds,
            depth,
            order,
            items,
            env: env.clone(),
            dismiss_requests,
            preview,
            accessory,
        });
    }
}
