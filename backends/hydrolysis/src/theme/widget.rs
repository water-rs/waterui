//! The widget chrome half of the theme contract: [`WidgetTheme`] and the
//! token tables, state snapshots and presentation plugins it names.

use cherenkov_record::{MaterialRegistry, Paint, Recorder};
use core::time::Duration;
use kurbo::{Point, Rect};
use nami::signal::IntoComputed;
use waterui::navigation::tab::NativeTabStyle;
use waterui_core::EasingCurve;
use waterui_core::animation::Animation;
use waterui_core::handler::SharedAction;
use waterui_core::interaction::InteractionState;
use waterui_core::plugin::Plugin;
use waterui_core::{Binding, Computed, Environment, Signal};
use waterui_graphics::color::Color;
use waterui_text::font::Font;

/// Publishes whether an interactive subtree owns keyboard focus.
#[derive(Debug, Clone)]
pub struct InteractionFocusBinding {
    focused: Binding<bool>,
    escape_action: Option<SharedAction>,
}

impl InteractionFocusBinding {
    /// Creates a focus observer backed by component-owned state.
    #[must_use]
    pub fn new(focused: &Binding<bool>) -> Self {
        Self {
            focused: focused.clone(),
            escape_action: None,
        }
    }

    /// Handles Escape while this interactive subtree owns keyboard focus.
    #[must_use]
    pub fn escape_action(mut self, action: SharedAction) -> Self {
        self.escape_action = Some(action);
        self
    }

    /// Returns the component-owned focus state.
    #[must_use]
    pub const fn focused(&self) -> &Binding<bool> {
        &self.focused
    }

    /// Returns the configured Escape action.
    #[must_use]
    pub const fn escape_action_handle(&self) -> Option<&SharedAction> {
        self.escape_action.as_ref()
    }
}

impl Plugin for InteractionFocusBinding {}

/// Marks a subtree as a modal interaction scope.
#[derive(Debug, Clone)]
pub struct ModalInteraction {
    active: Computed<bool>,
    close_on_escape: bool,
    escape_action: SharedAction,
}

impl ModalInteraction {
    /// Creates a modal scope with an optional Escape-key action.
    #[must_use]
    pub fn new(close_on_escape: bool, escape_action: SharedAction) -> Self {
        Self {
            active: Computed::constant(true),
            close_on_escape,
            escape_action,
        }
    }

    /// Controls whether the modal scope currently traps interaction.
    #[must_use]
    pub fn active(mut self, active: impl IntoComputed<bool>) -> Self {
        self.active = active.into_computed();
        self
    }

    /// Whether this modal scope currently traps interaction.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active.snapshot()
    }

    /// Whether Escape should dispatch the modal action.
    #[must_use]
    pub const fn close_on_escape(&self) -> bool {
        self.close_on_escape
    }

    /// Dispatches the modal Escape action.
    pub fn handle_escape(&self, env: &waterui_core::Environment) {
        self.escape_action.run(env);
    }
}

impl Plugin for ModalInteraction {}

/// Maximum number of press ripple waves tracked concurrently per widget.
///
/// Every press spawns its own wave (Material semantics), so rapid
/// re-presses overlap: older waves keep fading while the newest grows. A wave
/// lives for at most its grow plus fade-out duration (a few hundred
/// milliseconds), so this bound sits well above what any human click rate can
/// keep alive; when it is exceeded the oldest wave's slot is recycled.
pub const MAX_PRESS_WAVES: usize = 4;

/// One in-flight press ripple wave.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressWave {
    /// Absolute logical-unit origin of the press that spawned this wave.
    /// `None` centers the wave in the target (synthetic/keyboard presses).
    pub origin: Option<Point>,
    /// Animated grow progress in the 0.0..=1.0 range.
    pub progress: f32,
    /// Animated wave opacity sampled by the renderer.
    pub opacity: f32,
}

/// The press ripple waves currently visible on a widget, ordered oldest to
/// newest.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PressWaves {
    waves: [Option<PressWave>; MAX_PRESS_WAVES],
}

impl PressWaves {
    /// No visible press waves.
    pub const EMPTY: Self = Self {
        waves: [None; MAX_PRESS_WAVES],
    };

    /// Appends a wave as the newest entry, recycling the oldest slot when the
    /// set is full.
    pub fn push(&mut self, wave: PressWave) {
        if let Some(slot) = self.waves.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(wave);
            return;
        }
        self.waves.rotate_left(1);
        self.waves[MAX_PRESS_WAVES - 1] = Some(wave);
    }

    /// Iterates the visible waves from oldest to newest.
    pub fn iter(&self) -> impl Iterator<Item = PressWave> + '_ {
        self.waves.iter().filter_map(|wave| *wave)
    }

    /// Whether no wave is visible.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.waves.iter().all(Option::is_none)
    }

    /// The most recently spawned wave.
    #[must_use]
    pub fn latest(&self) -> Option<PressWave> {
        self.waves.iter().rev().find_map(|wave| *wave)
    }

    /// Maps every wave origin, e.g. from window space into a widget-local
    /// frame.
    pub fn map_origins(&mut self, mut map: impl FnMut(Point) -> Point) {
        for wave in self.waves.iter_mut().flatten() {
            wave.origin = wave.origin.map(&mut map);
        }
    }
}

/// Interactive state snapshot for widget chrome.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WidgetInteractionState {
    /// The widget's interaction flags for this frame — hovered, pressed,
    /// keyboard-focus-visible, dragged, selected, disabled — resolved by the
    /// renderer's bookkeeping. Themes read it to resolve
    /// [`StateValue`](waterui_core::interaction::StateValue) fields
    /// directly. When [`InteractionState::DISABLED`] is set the widget
    /// renders its inactive appearance and the renderer suppresses hover,
    /// press, and focus, so the remaining flags stay at rest.
    pub state: InteractionState,
    /// Animated focus affordance progress in the 0.0..=1.0 range.
    pub focus_progress: f32,
    /// Animated state-layer opacity sampled by the renderer.
    pub state_layer_opacity: f32,
    /// Animated physical-press progress in the 0.0..=1.0 range: moves
    /// towards 1 while the widget is physically pressed and towards 0 on
    /// release, with `InteractionMotion::press_shape` in both directions.
    /// Themes drive state-dependent shape morphs from it; unlike a ripple
    /// wave it plays backwards on release.
    pub press_progress: f32,
    /// The press ripple waves currently visible on the widget: one per press,
    /// so rapid re-presses overlap while older waves fade out.
    pub press_waves: PressWaves,
}

impl WidgetInteractionState {
    /// No active interaction state.
    pub const NONE: Self = Self {
        state: InteractionState::empty(),
        focus_progress: 0.0,
        state_layer_opacity: 0.0,
        press_progress: 0.0,
        press_waves: PressWaves::EMPTY,
    };
}

/// Motion policy for interactive widget chrome.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractionMotion {
    /// Hover state-layer opacity.
    pub hover_opacity: f32,
    /// Focus state-layer opacity.
    pub focus_opacity: f32,
    /// Pressed state-layer opacity.
    pub pressed_opacity: f32,
    /// Dragged state-layer opacity.
    pub dragged_opacity: f32,
    /// Animation used when hover appears.
    pub hover_enter: Animation,
    /// Animation used when hover disappears.
    pub hover_exit: Animation,
    /// Animation used when focus appears.
    pub focus_enter: Animation,
    /// Animation used when focus disappears.
    pub focus_exit: Animation,
    /// Animation used when the press layer fades in.
    pub press_fade_in: Animation,
    /// Animation used when the press layer fades out.
    pub press_fade_out: Animation,
    /// Animation used when the pressed ripple grows from origin to final size.
    pub press_grow: Animation,
    /// Animation used for `WidgetInteractionState::press_progress` in both
    /// directions.
    pub press_shape: Animation,
    /// Minimum amount of time a press must remain visible.
    pub minimum_press_duration: Duration,
    /// Touch delay before showing a press ripple.
    pub touch_delay: Duration,
}

/// Motion policy for text input carets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextCaretMotion {
    /// Full fade cycle duration.
    pub fade_cycle_duration: Duration,
    /// Time between caret animation redraws.
    pub frame_interval: Duration,
    /// Minimum opacity at the dimmest point of the fade cycle.
    pub min_opacity: f32,
}

/// Motion policy for backend-rendered navigation transitions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavigationMotion {
    /// Duration for a complete navigation transition.
    pub transition_duration: Duration,
    /// Easing applied to programmatic navigation progress.
    pub transition_easing: EasingCurve,
    /// Absolute travel distance for the Material shared-axis X pattern.
    pub shared_axis_slide_distance: f64,
    /// Progress at which the outgoing layer has faded out and the incoming
    /// layer begins fading in.
    pub fade_through_threshold: f32,
}

/// Text input layout metrics.
#[derive(Debug, Clone, Copy)]
pub struct InputFieldMetrics {
    /// Height reserved for the input label.
    pub label_height: f64,
    /// Minimum input field width.
    pub min_width: f64,
    /// Minimum input field height.
    pub min_height: f64,
    /// Horizontal text inset.
    pub horizontal_inset: f64,
    /// Vertical text inset.
    pub vertical_inset: f64,
}

impl InputFieldMetrics {
    /// Create text input layout metrics.
    #[must_use]
    pub const fn new(
        label_height: f64,
        min_width: f64,
        min_height: f64,
        horizontal_inset: f64,
        vertical_inset: f64,
    ) -> Self {
        Self {
            label_height,
            min_width,
            min_height,
            horizontal_inset,
            vertical_inset,
        }
    }
}

/// How a tab bar lays out each item's icon and label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each tab item arrangement variant by variant; a new tab item arrangement must fail their build instead of falling into a wildcard arm"
)]
pub enum TabItemLayout {
    /// The icon above the label, the indicator behind the icon: the compact
    /// bar of a narrow window.
    Vertical,
    /// The icon beside the label, the indicator behind both: the bar of a
    /// medium-width window, where items have room to spread.
    Horizontal,
}

/// The geometry of one tab item, in the coordinate space of the bar that
/// holds it: what a style needs to draw the selection indicator wherever and
/// in whatever shape it wants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabItemGeometry {
    /// The item's chrome bounds: the whole button for vertical items, the
    /// icon and title grown by the button inset for horizontal ones.
    pub bounds: Rect,
    /// The icon's rect, `None` for a title-only item.
    pub icon: Option<Rect>,
    /// The title's rect.
    pub title: Rect,
    /// How the item arranges its icon and title.
    pub layout: TabItemLayout,
    /// Whether the item sits in a bar or a sidebar strip; `Automatic` and
    /// `TabBar` are both a bar.
    pub placement: NativeTabStyle,
}

/// Layout metrics for tab containers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabsMetrics {
    /// Tab bar height.
    pub bar_height: f64,
    /// Minimum width for one tab button.
    pub button_min_width: f64,
    /// Horizontal label inset inside one tab button.
    pub button_horizontal_inset: f64,
    /// Gap between a tab item's icon and its label: vertical space when the
    /// icon sits above the label, horizontal space when it sits beside it.
    pub icon_label_spacing: f64,
}

impl TabsMetrics {
    /// Create tab layout metrics.
    #[must_use]
    pub const fn new(
        bar_height: f64,
        button_min_width: f64,
        button_horizontal_inset: f64,
        icon_label_spacing: f64,
    ) -> Self {
        Self {
            bar_height,
            button_min_width,
            button_horizontal_inset,
            icon_label_spacing,
        }
    }
}

/// Layout metrics for lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListMetrics {
    /// Minimum one-line row height.
    pub one_line_row_height: f64,
    /// Horizontal content inset.
    pub horizontal_inset: f64,
    /// Vertical content inset.
    pub vertical_inset: f64,
    /// Divider leading inset.
    pub divider_leading_inset: f64,
    /// Divider trailing inset.
    pub divider_trailing_inset: f64,
    /// Width of the row move affordance.
    pub move_control_width: f64,
    /// Width of the row delete affordance.
    pub delete_control_width: f64,
    /// Gap between trailing row affordances.
    pub trailing_control_spacing: f64,
    /// Vertical inset for trailing row affordances.
    pub trailing_control_vertical_inset: f64,
    /// Height reserved above a row that opens a titled section.
    pub section_header_height: f64,
    /// Height reserved below the last row of a section that has a footer.
    pub section_footer_height: f64,
}

/// Row metrics for lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListRowMetrics {
    /// Minimum one-line row height.
    pub one_line_row_height: f64,
    /// Horizontal content inset.
    pub horizontal_inset: f64,
    /// Vertical content inset.
    pub vertical_inset: f64,
}

impl ListRowMetrics {
    /// Create list row layout metrics.
    #[must_use]
    pub const fn new(one_line_row_height: f64, horizontal_inset: f64, vertical_inset: f64) -> Self {
        Self {
            one_line_row_height,
            horizontal_inset,
            vertical_inset,
        }
    }
}

/// Divider metrics for lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListDividerMetrics {
    /// Divider leading inset.
    pub leading_inset: f64,
    /// Divider trailing inset.
    pub trailing_inset: f64,
}

impl ListDividerMetrics {
    /// Create list divider layout metrics.
    #[must_use]
    pub const fn new(leading_inset: f64, trailing_inset: f64) -> Self {
        Self {
            leading_inset,
            trailing_inset,
        }
    }
}

/// Trailing control metrics for lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListTrailingControlMetrics {
    /// Width of the row move affordance.
    pub move_width: f64,
    /// Width of the row delete affordance.
    pub delete_width: f64,
    /// Gap between trailing row affordances.
    pub spacing: f64,
    /// Vertical inset for trailing row affordances.
    pub vertical_inset: f64,
}

impl ListTrailingControlMetrics {
    /// Create list trailing control layout metrics.
    #[must_use]
    pub const fn new(
        move_width: f64,
        delete_width: f64,
        spacing: f64,
        vertical_inset: f64,
    ) -> Self {
        Self {
            move_width,
            delete_width,
            spacing,
            vertical_inset,
        }
    }
}

impl ListMetrics {
    /// Create list layout metrics.
    #[must_use]
    pub const fn new(
        row: ListRowMetrics,
        divider: ListDividerMetrics,
        trailing_controls: ListTrailingControlMetrics,
        sections: ListSectionMetrics,
    ) -> Self {
        Self {
            one_line_row_height: row.one_line_row_height,
            horizontal_inset: row.horizontal_inset,
            vertical_inset: row.vertical_inset,
            divider_leading_inset: divider.leading_inset,
            divider_trailing_inset: divider.trailing_inset,
            move_control_width: trailing_controls.move_width,
            delete_control_width: trailing_controls.delete_width,
            trailing_control_spacing: trailing_controls.spacing,
            trailing_control_vertical_inset: trailing_controls.vertical_inset,
            section_header_height: sections.header_height,
            section_footer_height: sections.footer_height,
        }
    }
}

/// Section chrome metrics for lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListSectionMetrics {
    /// Height reserved above a row that opens a titled section.
    pub header_height: f64,
    /// Height reserved below the last row of a section that has a footer.
    pub footer_height: f64,
}

impl ListSectionMetrics {
    /// Create list section chrome metrics.
    #[must_use]
    pub const fn new(header_height: f64, footer_height: f64) -> Self {
        Self {
            header_height,
            footer_height,
        }
    }
}

/// Layout metrics for dividers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DividerMetrics {
    /// Divider stroke thickness.
    pub thickness: f64,
}

impl DividerMetrics {
    /// Create divider layout metrics.
    #[must_use]
    pub const fn new(thickness: f64) -> Self {
        Self { thickness }
    }
}

/// Layout metrics for badges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BadgeMetrics {
    /// Diameter of a small dot badge.
    pub small_size: f64,
    /// Height and minimum width of a large labeled badge.
    pub large_size: f64,
    /// Horizontal padding inside a large labeled badge.
    pub large_horizontal_padding: f64,
    /// Inset of the badge's leading edge from the content's trailing edge for
    /// a small badge. Renderers place the badge's leading edge at
    /// `trailing_edge - small_offset_x` in left-to-right layout and mirror it
    /// to the leading edge in right-to-left layout.
    pub small_offset_x: f64,
    /// Downward overlap of the badge's bottom edge past the content's top edge
    /// for a small badge: the badge's bottom sits at `top + small_offset_y`.
    pub small_offset_y: f64,
    /// Inset of the badge's leading edge from the content's trailing edge for
    /// a large badge; mirrors to the leading edge in right-to-left layout.
    pub large_offset_x: f64,
    /// Downward overlap of the badge's bottom edge past the content's top edge
    /// for a large badge: the badge's bottom sits at `top + large_offset_y`.
    pub large_offset_y: f64,
}

impl BadgeMetrics {
    /// Create badge layout metrics.
    #[must_use]
    pub const fn new(
        small_size: f64,
        large_size: f64,
        large_horizontal_padding: f64,
        small_offset_x: f64,
        small_offset_y: f64,
        large_offset_x: f64,
        large_offset_y: f64,
    ) -> Self {
        Self {
            small_size,
            large_size,
            large_horizontal_padding,
            small_offset_x,
            small_offset_y,
            large_offset_x,
            large_offset_y,
        }
    }
}

/// Layout metrics for navigation bars.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavigationMetrics {
    /// Automatic title-mode bar height.
    pub automatic_bar_height: f64,
    /// Inline title-mode bar height.
    pub inline_bar_height: f64,
    /// Medium title-mode bar height.
    pub medium_bar_height: f64,
    /// Large title-mode bar height.
    pub large_bar_height: f64,
    /// Navigation title slot height for inline bars.
    pub inline_title_height: f64,
    /// Navigation title slot height for medium bars.
    pub medium_title_height: f64,
    /// Navigation title slot height for large bars.
    pub large_title_height: f64,
    /// Title leading inset when no leading bar item is present.
    pub title_leading_inset: f64,
    /// Title trailing inset when no trailing bar item is present.
    pub title_trailing_inset: f64,
    /// Bottom inset for large navigation titles.
    pub large_title_bottom_inset: f64,
    /// Horizontal leading/trailing bar inset.
    pub horizontal_inset: f64,
    /// Gap between title and adjacent items.
    pub item_spacing: f64,
    /// Search field height.
    pub search_height: f64,
    /// Vertical search inset.
    pub search_vertical_inset: f64,
    /// Navigation back button size.
    pub back_button_size: f64,
    /// Navigation back button leading inset.
    pub back_button_leading_inset: f64,
    /// Navigation back button top inset.
    pub back_button_top_inset: f64,
}

/// Layout metrics for backend-rendered tables.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableMetrics {
    /// Minimum table column width.
    pub min_column_width: f64,
    /// Horizontal padding added to measured cell content.
    pub cell_horizontal_padding: f64,
    /// Inset used when placing header and row cell content.
    pub cell_vertical_inset: f64,
    /// Header row height.
    pub header_height: f64,
    /// Data row height.
    pub row_height: f64,
    /// Table outline width.
    pub outline_width: f64,
}

/// The widget chrome a style package supplies to Hydrolysis.
///
/// Hydrolysis lays out every widget it renders from the theme's metrics,
/// animates it with the theme's motion policies, and records its chrome by
/// calling the theme's `draw_*` methods with the widget's bounds and the
/// renderer-resolved [`WidgetInteractionState`]. Every draw method records
/// into a `cherenkov-record` [`Recorder`], the render-target-neutral
/// vocabulary every Hydrolysis render target consumes, so a theme never
/// names a render engine. A theme becomes a runnable style by also
/// implementing [`Style`](crate::Style).
pub trait WidgetTheme {
    /// Return motion policy for interactive widget chrome.
    fn interaction_motion(&self) -> InteractionMotion;
    /// Return motion policy for navigation transitions.
    fn navigation_motion(&self) -> NavigationMotion;

    /// Declare the theme's backdrop materials in `registry`: both the
    /// backdrop shaders ([`MaterialShader`] keys, each with its
    /// [`BackdropShaderSource`]) and the capture classes ([`CaptureClass`]
    /// keys, each with its [`MaterialCapture`] group parameters) that the
    /// theme's chrome names when it records a backdrop material with
    /// [`Recorder::backdrop_material`].
    ///
    /// [`MaterialShader`]: cherenkov_record::MaterialShader
    /// [`BackdropShaderSource`]: cherenkov_record::BackdropShaderSource
    /// [`CaptureClass`]: cherenkov_record::CaptureClass
    /// [`MaterialCapture`]: cherenkov_record::MaterialCapture
    ///
    /// The theme also registers here a [`ContainerRecipe`] for each shader
    /// whose members under one `.material_group()` scope are drawn as one
    /// container: the backend resolves such a group through the recipe,
    /// which names the group's capture class from all its members and
    /// gives each member its effect, instead of grouping the members by
    /// the class each recorded.
    ///
    /// [`ContainerRecipe`]: cherenkov_record::ContainerRecipe
    ///
    /// The contract a backend that realizes backdrop materials follows: it
    /// calls this once, before it draws any chrome, and keeps the
    /// registry; it registers every shader with each engine it attaches
    /// and creates material groups from the capture classes, or from the
    /// class a container recipe resolves. The theme
    /// holds no engine handles. The default registers nothing, which is
    /// right for a theme that records no backdrop material.
    fn register_backdrop_shaders(&self, _registry: &mut MaterialRegistry) {}

    /// The look of `glass` behind content; `None` draws no glass (the
    /// default).
    ///
    /// Hydrolysis requests the look once for each `.background(Glass…)` the
    /// first time it draws it, in the environment the content resolves in,
    /// and realizes it: the outline is `glass`'s shape — the rectangle
    /// family, a capsule, a circle, an ellipse or a custom path — on the
    /// content's frame, following the frame as it changes, and the [`GlassLook`](super::GlassLook) states what
    /// fills it. The look's [`Backdrop`](super::Backdrop) is recorded behind
    /// the content as a backdrop material, with the border and the
    /// elevation as a [`Surface`](super::Surface) records them, in the
    /// nearest enclosing `.material_group()` scope — two glass backgrounds
    /// under one scope share one capture, and outside every scope each is
    /// a group of its own. The material's shape and effect, the border and
    /// the elevation are live operands, so a frame or input change records
    /// nothing.
    ///
    /// The glass takes no part in hit testing: it never intercepts a tap,
    /// so the content and the views beneath it receive their input as if
    /// there were no glass, and it takes no keyboard focus. A glass that is
    /// [`interactive`](waterui::background::Glass::is_interactive) shows
    /// press feedback through its [`Driven`](super::Driven) values, which
    /// read the interaction state and the press point: a primary press
    /// inside the content's frame sets `PRESSED` and the press point
    /// while it is held — whichever view the press goes to, a button inside
    /// the content included — and moves the point with the pointer. A glass
    /// that is not interactive reads the inputs at rest.
    ///
    /// The tint is [`Glass::tint_color`](waterui::background::Glass::tint_color),
    /// a live [`Color`] the look resolves through `env` (`Color::resolve`);
    /// the backend does nothing with it.
    ///
    /// Material-bearing parts need the theme's shaders and capture classes
    /// registered by [`Self::register_backdrop_shaders`].
    fn glass_background(
        &self,
        _glass: &waterui::background::Glass,
        _env: &Environment,
    ) -> Option<super::GlassLook> {
        None
    }

    /// The look of a button: a `Button`, an icon-only button and a menu
    /// trigger.
    ///
    /// Hydrolysis measures, places, hit-tests, records and animates the
    /// button from the returned look, and requests it again only when
    /// `spec` changes; everything else reaches the look through its
    /// [`Driven`](super::Driven) values. A button inside a composed
    /// control's [`Chrome`](super::Chrome) plugin takes that chrome, with
    /// no padding, no minimum size and its label unstyled; a button on a
    /// floating surface keeps this look's state-layer tokens, press
    /// feedback, focus indicator, motion and hit region over the floating
    /// style's container, colours and metrics.
    fn button(&self, spec: &super::ButtonSpec, env: &Environment) -> super::ButtonLook;

    /// Content alpha for labels accompanying a disabled control (Material's
    /// 38% disabled-content token). Backends dim a disabled control's label
    /// views with this alpha.
    fn disabled_content_alpha(&self) -> f32 {
        0.38
    }

    /// The look of a switch toggle.
    ///
    /// Hydrolysis measures, places, hit-tests, records and animates the
    /// switch from the returned look, and requests it again only when `spec`
    /// changes; everything else reaches the look through its
    /// [`Driven`](super::Driven) values, which read the toggle's `bool`, so a
    /// toggle, hover, press or drag moves live operands and records nothing.
    /// The look's [`SwitchDrag`](super::SwitchDrag) maps a drag of the knob
    /// to the value it settles on.
    fn switch(&self, spec: &super::ToggleSpec, env: &Environment) -> super::SwitchLook;

    /// The look of a checkbox toggle, requested and realized like
    /// [`Self::switch`] but taking taps only.
    fn checkbox(&self, spec: &super::ToggleSpec, env: &Environment) -> super::CheckboxLook;

    /// The look of a stepper.
    ///
    /// Hydrolysis measures, places, hit-tests and records the stepper from
    /// the returned look, and requests it again only when `spec` changes;
    /// everything else reaches the look through its ends'
    /// [`Chrome`](super::Chrome) values, so a press, a hover or a value
    /// change moves live operands and records nothing. An end at the limit
    /// of the stepper's range is disabled.
    fn stepper(&self, spec: &super::StepperSpec, env: &Environment) -> super::StepperLook;

    /// The look of a text field, and of a secure field.
    ///
    /// Hydrolysis measures, places and records the field from the returned
    /// look, and requests it again only when `spec` changes; everything else
    /// reaches the look through its [`Chrome`](super::Chrome) and
    /// [`Driven`](super::Driven) values, so a hover, a focus change or the
    /// label floating moves live operands and records nothing.
    fn text_field(&self, spec: &super::TextFieldSpec, env: &Environment) -> super::TextFieldLook;

    /// The look of every menu presentation: the picker popup, the text
    /// context menu and the popup window a menu button opens.
    ///
    /// Hydrolysis measures, places and hit-tests a menu from the returned
    /// look's layout and records its panel, separators and selected row
    /// from its parts, and requests it again only when `spec` changes.
    fn menu(&self, spec: &super::MenuSpec, env: &Environment) -> super::MenuLook;

    /// The look of a menu picker's field.
    ///
    /// Hydrolysis measures, places, hit-tests and records the field from
    /// the returned look, and requests it again only when `spec` changes;
    /// the popup the field opens is the [`menu`](Self::menu) look.
    fn menu_picker(&self, spec: &super::PickerSpec, env: &Environment) -> super::MenuPickerLook;

    /// The look of a radio group: the indicator each item draws and the
    /// measures the group is laid out by.
    ///
    /// Each item's indicator is live over whether the item is selected, so
    /// a selection change moves the operands of the two items it touches
    /// and records nothing.
    fn radio_group(&self, spec: &super::PickerSpec, env: &Environment) -> super::RadioGroupLook;

    /// The look of a segmented picker.
    ///
    /// Hydrolysis measures, places, hit-tests, records and animates the
    /// picker from the returned look, and requests it again only when
    /// `spec` changes; everything else reaches the look through its
    /// [`Driven`](super::Driven) values, which read the picker's
    /// [`SegmentedValue`](super::SegmentedValue) or each segment's
    /// [`Segment`](super::Segment), so a selection change, a hover, a press
    /// or a drag moves live operands and records nothing.
    fn segmented_picker(
        &self,
        spec: &super::SegmentedSpec,
        env: &Environment,
    ) -> super::SegmentedLook;

    /// Return how the slider of `spec` looks and responds, as plain data
    /// (water-rs/waterui#1434): the measures it is laid out by, its track,
    /// fill, knob and value indicator as parts over the normalized value,
    /// the motion of that value and how a drag maps to it. Hydrolysis
    /// measures, places, hit-tests, records and animates the slider from
    /// the look alone.
    fn slider(&self, spec: &super::SliderSpec, env: &Environment) -> super::SliderLook;

    /// The look of a linear progress indicator, determinate or not.
    ///
    /// Hydrolysis measures, places, records and animates the indicator from
    /// the returned look, and requests it again only when `spec` changes;
    /// the value reaches the look through its [`Driven`](super::Driven)
    /// values, which follow it through the look's value motion. An
    /// indeterminate indicator's motion is the look's repeating values and
    /// rotating parts, which the engine runs.
    fn linear_progress(
        &self,
        spec: &super::ProgressSpec,
        env: &Environment,
    ) -> super::LinearProgressLook;
    /// The look of a circular progress indicator, determinate or not.
    fn circular_progress(
        &self,
        spec: &super::ProgressSpec,
        env: &Environment,
    ) -> super::IndicatorLook;
    /// The look of a loading indicator: an indeterminate indicator that says
    /// work is under way, with no reading to fill.
    fn loading_indicator(
        &self,
        spec: &super::ProgressSpec,
        env: &Environment,
    ) -> super::IndicatorLook;

    /// Return navigation bar layout metrics.
    fn navigation_metrics(&self) -> NavigationMetrics;
    /// Draw a navigation bar background.
    fn draw_navigation_bar(&self, recorder: &mut Recorder, bounds: Rect, background: &Paint);
    /// Draw a navigation bar separator.
    fn draw_navigation_bar_separator(&self, recorder: &mut Recorder, bounds: Rect);
    /// Draw a navigation back button.
    fn draw_navigation_back_button(&self, recorder: &mut Recorder, bounds: Rect);

    /// How the tab bar lays out its items when the bar itself is
    /// `bar_width` wide and holds `item_count` items. `bar_width` is the
    /// bar's own extent, not the tab container's: a sidebar strip passes its
    /// strip width. Themes without a horizontal layout keep the default.
    fn tabs_item_layout(&self, bar_width: f64, item_count: usize) -> TabItemLayout {
        let _ = (bar_width, item_count);
        TabItemLayout::Vertical
    }
    /// Return tabs layout metrics for items laid out as `layout`.
    fn tabs_metrics(&self, layout: TabItemLayout) -> TabsMetrics;
    /// Draw a tabs bar.
    ///
    /// `surface` is the bar's surface already extended through the safe-area
    /// regions of every edge it touches.
    fn draw_tabs_bar(&self, recorder: &mut Recorder, surface: Rect);
    /// Draw a tabs bar divider.
    ///
    /// `divider` is the bar's inner edge: the top row of a bottom bar, the
    /// trailing column of a sidebar strip.
    fn draw_tabs_bar_divider(&self, recorder: &mut Recorder, divider: Rect);
    /// Draw the selected tab's indicator. The style decides its shape and
    /// placement from the item's geometry.
    fn draw_tabs_highlight(&self, recorder: &mut Recorder, item: TabItemGeometry);
    /// Draw the state layer of a tab button. The style decides its shape and
    /// placement from the item's geometry, like the selection indicator.
    fn draw_tabs_button_state_layer(
        &self,
        _recorder: &mut Recorder,
        _item: TabItemGeometry,
        _selected: bool,
        _state: WidgetInteractionState,
    ) {
    }

    /// Draw a scroll indicator.
    fn draw_scroll_indicator(&self, recorder: &mut Recorder, bounds: Rect);

    /// Return divider layout metrics.
    fn divider_metrics(&self) -> DividerMetrics;
    /// Draw a divider.
    fn draw_divider(&self, recorder: &mut Recorder, bounds: Rect);

    /// Return badge layout metrics.
    fn badge_metrics(&self) -> BadgeMetrics;
    /// Return badge label foreground color.
    fn badge_label_color(&self) -> Color;
    /// Return badge label font.
    fn badge_label_font(&self) -> Font;
    /// Draw a small dot badge.
    fn draw_badge_small(&self, recorder: &mut Recorder, bounds: Rect);
    /// Draw a large labeled badge.
    fn draw_badge_large(&self, recorder: &mut Recorder, bounds: Rect);

    /// Return list layout metrics.
    fn list_metrics(&self) -> ListMetrics;
    /// Draw a list row background.
    fn draw_list_row_background(&self, recorder: &mut Recorder, bounds: Rect, alternate: bool);
    /// Draw a list move affordance.
    fn draw_list_move_control(&self, recorder: &mut Recorder, bounds: Rect);
    /// Draw a list move affordance state layer.
    fn draw_list_move_control_state_layer(
        &self,
        _recorder: &mut Recorder,
        _bounds: Rect,
        _state: WidgetInteractionState,
    ) {
    }
    /// Draw a list delete affordance.
    fn draw_list_delete_control(&self, recorder: &mut Recorder, bounds: Rect);
    /// Draw a list delete affordance state layer.
    fn draw_list_delete_control_state_layer(
        &self,
        _recorder: &mut Recorder,
        _bounds: Rect,
        _state: WidgetInteractionState,
    ) {
    }
    /// Draw the background revealed behind a row that is being swiped away.
    ///
    /// `progress` runs `0.0..=1.0` toward the dismiss threshold, and
    /// `toward_start` is `true` while the row travels toward the leading edge,
    /// so a theme can anchor its icon on the side the row is uncovering.
    fn draw_list_swipe_dismiss_background(
        &self,
        _recorder: &mut Recorder,
        _bounds: Rect,
        _progress: f64,
        _toward_start: bool,
    ) {
    }
    /// Draw the lifted treatment for a row being dragged to a new position.
    fn draw_list_row_lifted(&self, _recorder: &mut Recorder, _bounds: Rect, _elevation: f64) {}
    /// Draw a list separator.
    fn draw_list_separator(&self, recorder: &mut Recorder, bounds: Rect);

    /// Return table layout metrics.
    fn table_metrics(&self) -> TableMetrics;
    /// Draw a table background.
    fn draw_table_background(&self, recorder: &mut Recorder, bounds: Rect);
    /// Draw a table header background.
    fn draw_table_header_background(&self, recorder: &mut Recorder, bounds: Rect);
    /// Draw a table cell border.
    fn draw_table_cell_border(&self, recorder: &mut Recorder, bounds: Rect);
    /// Draw a table column separator.
    fn draw_table_column_separator(&self, recorder: &mut Recorder, from: Point, to: Point);
}
