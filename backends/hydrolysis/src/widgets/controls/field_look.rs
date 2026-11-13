//! Hydrolysis's realization of the text field look
//! ([`TextFieldLook`](crate::theme::TextFieldLook)): what a text field, a
//! secure field and the pickers that wear a field's container keep across
//! renders, and the operands the floating label rides.
//!
//! The look is requested once per spec and held. Its container is realized
//! as live chrome, so a hover, a focus change or the label floating moves
//! operands and records nothing.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cherenkov_record::Paint;
use kurbo::{Affine, Rect};
use nami::{Binding, Signal as _, SignalExt as _};
use waterui::ViewExt as _;
use waterui_core::animation::{Animation, ChangeAnimation};
use waterui_core::interaction::InteractionState;
use waterui_core::{AnyView, Environment, View};
use waterui_graphics::color::{Color, signal_color};
use waterui_text::styled::StyledStr;

use crate::renderer::chrome::{ChromeContent, LiveChromeParts};
use crate::renderer::{
    InteractionKey, InteractionLayerHandles, PressSlot, RetainedSubview, SemanticCore,
    WidgetRenderContext,
};
use crate::theme::live::Operand;
use crate::theme::{
    Border, Chrome, Custom, Driven, Inputs, SpecTint, TextFieldLook, TextFieldSpec, ValueMotion,
    WidgetInteractionState, WidgetTheme,
};
use crate::widgets::controls::button::LIVE_STATES;
use crate::widgets::controls::button::styled_button_label;
use waterui_controls::label::Label;

/// The spec of a field in `env`.
pub fn field_spec(env: &Environment) -> TextFieldSpec {
    TextFieldSpec::new(
        waterui_core::layout::layout_direction(env).snapshot(),
        SpecTint::resolve(env),
    )
}

/// The look `theme` gives a field in `env`, for a measurement that holds
/// none.
pub fn field_look(theme: &Rc<dyn WidgetTheme>, env: &Environment) -> TextFieldLook {
    theme.text_field(&field_spec(env), env)
}

/// The container's chrome with the look's active indicator drawn along its
/// bottom edge, over the surface and beneath the state layer.
fn container_of(look: &TextFieldLook) -> Chrome {
    let chrome = look.container().clone();
    let Some(indicator) = look.get_active_indicator() else {
        return chrome;
    };
    chrome.custom(indicator_part(indicator))
}

/// The active indicator as a custom part of the container: a line of the
/// border's width along the frame's bottom edge, painted with its paint.
fn indicator_part(indicator: &Driven<Border>) -> Custom<(), (Driven<Paint>, Driven<f64>)> {
    Custom::new(
        (
            indicator.map(|border| border.paint),
            indicator.map(|border| border.width),
        ),
        |recorder, frame, (paint, width)| {
            use cherenkov_record::Draw as _;
            recorder.fill(
                width.map(move |width| Rect::new(frame.x0, frame.y1 - width, frame.x1, frame.y1)),
                paint,
            );
        },
    )
}

/// The look a field was requested for, and the container it realizes.
struct Held {
    spec: TextFieldSpec,
    look: TextFieldLook,
    container: Chrome,
}

/// The colour the floating label's view takes, read through the field's
/// reported state: the look's, when it has one.
type LabelColor = Rc<RefCell<Option<Driven<Color>>>>;

/// What a field keeps across renders to realize its look: the look for the
/// spec it was requested for, the container's live chrome and the
/// reported-state binding that keeps the floating label's operands and
/// colour current.
pub struct FieldChrome {
    held: Option<Held>,
    parts: Option<LiveChromeParts>,
    report: Binding<InteractionState>,
    float: Rc<FloatLabel>,
    /// Whether the look was taken anew since the label view last took its
    /// font.
    label_font_stale: bool,
    /// Whether the label view carries a look font rather than the config
    /// label's own.
    label_font_set: bool,
    label_color: LabelColor,
    /// The subscription that floats the label with the field's focus.
    _focus: Box<dyn Any>,
}

impl FieldChrome {
    pub(crate) fn new() -> Self {
        let report = Binding::container(InteractionState::empty());
        let float = Rc::new(FloatLabel::new());
        let focus = {
            let (float, state) = (Rc::clone(&float), report.clone());
            report.watch(move |_| float.focus(state.snapshot().contains(InteractionState::FOCUSED)))
        };
        Self {
            held: None,
            parts: None,
            report,
            float,
            label_font_stale: true,
            label_font_set: false,
            label_color: Rc::default(),
            _focus: Box::new(focus),
        }
    }

    /// `label` as the field's inside label view: it takes the look's label
    /// colour for the field's reported state as its default, else the
    /// ordinary foreground, and follows the state without rebuilding.
    pub(crate) fn label_view(&self, label: impl View + 'static) -> AnyView {
        let color = Rc::clone(&self.label_color);
        let color = signal_color(self.report.clone().map(move |state| {
            color.borrow().as_ref().map_or_else(
                || Color::new(waterui::theme::color::Foreground),
                |color| color.target(&Inputs::at_rest(Rect::ZERO, state, &())),
            )
        }));
        AnyView::new(label.foreground(color))
    }

    /// Gives the inside label view the font of the look the field took last.
    ///
    /// The view is built before the theme is reachable, so the render that
    /// takes a look builds it again from `label`: with the look's label font
    /// when it names one, else with the config label's own font. The field
    /// lays out again, so measurement uses the drawn font. A look the field
    /// already applied changes nothing.
    pub(crate) fn apply_label_font<E: crate::text::TextEngine>(
        &mut self,
        label_view: &mut RetainedSubview<E>,
        label: &Label,
        renderer: &mut SemanticCore<E>,
        env: &Environment,
    ) {
        if !core::mem::take(&mut self.label_font_stale) {
            return;
        }
        let font = self
            .held
            .as_ref()
            .expect("a field applies the look it requested")
            .look
            .floating_label()
            .label()
            .get_font()
            .map(|font| font.target(&Inputs::at_rest(Rect::ZERO, InteractionState::empty(), &())));
        if font.is_none() && !self.label_font_set {
            return;
        }
        self.label_font_set = font.is_some();
        *label_view =
            RetainedSubview::new(self.label_view(styled_button_label(font, label.clone())));
        label_view.ensure_built(renderer, env);
        renderer.context_mark_layout();
    }

    /// The look for the field's spec in `env`: the one already held when the
    /// spec is unchanged, otherwise requested again.
    pub(crate) fn look(
        &mut self,
        theme: &Rc<dyn WidgetTheme>,
        env: &Environment,
    ) -> &TextFieldLook {
        let spec = field_spec(env);
        if self.held.as_ref().is_none_or(|held| held.spec != spec) {
            let look = theme.text_field(&spec, env);
            let container = container_of(&look);
            self.label_font_stale = true;
            self.held = Some(Held {
                spec,
                look,
                container,
            });
        }
        let held = self.held.as_ref().expect("the look was just ensured");
        self.float
            .set_motion(held.look.floating_label().get_float().clone());
        *self.label_color.borrow_mut() = held.look.floating_label().label().get_color().cloned();
        &held.look
    }

    /// The look for the field's spec in `env` without holding it: the held
    /// one, or a fresh one for a measurement ahead of the render that holds
    /// it.
    pub(crate) fn peek(&self, theme: &Rc<dyn WidgetTheme>, env: &Environment) -> TextFieldLook {
        let spec = field_spec(env);
        self.held
            .as_ref()
            .filter(|held| held.spec == spec)
            .map_or_else(|| theme.text_field(&spec, env), |held| held.look.clone())
    }

    /// Binds the field's interaction target on `bounds`, realizes the
    /// container's chrome there and lists its layers behind the field's
    /// content. Answers the interaction the field reports, the press slot a
    /// field that takes presses registers and the handles that steer the
    /// chrome. [`Self::look`] ran first.
    pub(crate) fn bind<E: crate::text::TextEngine>(
        &mut self,
        ctx: &mut WidgetRenderContext<'_, E>,
        key: &InteractionKey,
        bounds: Rect,
        env: &Environment,
        disabled: bool,
        focused: bool,
    ) -> (
        WidgetInteractionState,
        PressSlot,
        Rc<InteractionLayerHandles>,
    ) {
        let container = self
            .held
            .as_ref()
            .expect("a field binds the look it requested")
            .container
            .clone();
        let target = container.target(bounds);
        let (interaction, press_slot, handles) = ctx.renderer_mut().bind_live_field_target(
            key.clone(),
            target,
            env,
            disabled,
            &container,
            focused,
        );
        let flags = interaction.state;
        // Feed the state the field binds in to the floating label's colour
        // signal: the chrome reports only the changes it syncs, so a state
        // the field mounts in — disabled above all — reaches the label here.
        if self.report.snapshot() != flags {
            self.report.set(flags);
        }
        let parts = LiveChromeParts::ensure(
            &mut self.parts,
            &container,
            &ChromeContent::default(),
            bounds,
            flags,
            env,
            &self.report,
        )
        .clone();
        handles.attach_live(Rc::clone(parts.chrome()));
        let render_ctx = ctx.render_context();
        parts.push_below(ctx.renderer_mut(), render_ctx);
        parts.push_above(ctx.renderer_mut(), render_ctx);
        (interaction, press_slot, handles)
    }

    /// The floating label's operands.
    pub(crate) fn float(&self) -> &FloatLabel {
        &self.float
    }
}

/// The operands the inside label rides: its transform from resting to
/// floating, and the opacity of the text it makes way for.
///
/// The label floats while the field is focused or holds content. A change
/// of either moves both operands under the look's float motion, so the
/// label floating records nothing.
pub struct FloatLabel {
    focused: Cell<bool>,
    filled: Cell<bool>,
    /// Whether the operands were placed once: the first placement snaps.
    placed: Cell<bool>,
    /// The label's transform over its resting place when it floats.
    floated: Cell<Affine>,
    motion: RefCell<Option<ValueMotion>>,
    transform: Operand<Affine>,
    opacity: Operand<f32>,
}

impl FloatLabel {
    fn new() -> Self {
        Self {
            focused: Cell::new(false),
            filled: Cell::new(false),
            placed: Cell::new(false),
            floated: Cell::new(Affine::IDENTITY),
            motion: RefCell::new(None),
            transform: Operand::new(Affine::IDENTITY),
            opacity: Operand::new(0.0),
        }
    }

    fn set_motion(&self, motion: ValueMotion) {
        *self.motion.borrow_mut() = Some(motion);
    }

    /// Whether the label rests at the top.
    pub(crate) const fn is_floated(&self) -> bool {
        self.focused.get() || self.filled.get()
    }

    /// The animation a change of the float runs with.
    fn animation(&self) -> Option<Animation> {
        self.motion
            .borrow()
            .as_ref()
            .and_then(|motion| motion.animation(ChangeAnimation::Unspecified))
    }

    fn settle(&self, animation: Option<&Animation>) {
        let (transform, opacity) = if self.is_floated() {
            (self.floated.get(), 1.0)
        } else {
            (Affine::IDENTITY, 0.0)
        };
        self.transform.set(transform, animation);
        self.opacity.set(opacity, animation);
    }

    /// Floats the label with the field's focus between renders.
    fn focus(&self, focused: bool) {
        if self.focused.replace(focused) == focused {
            return;
        }
        self.settle(self.animation().as_ref());
    }

    /// Places the label for a render: the field is `focused` or `filled`,
    /// and the label floats by `floated` over its resting place. The first
    /// placement and a move of the floating place snap; a change of whether
    /// the label floats runs the look's motion.
    pub(crate) fn place(&self, focused: bool, filled: bool, floated: Affine) {
        let first = !self.placed.replace(true);
        let moved = self.floated.replace(floated) != floated;
        let snaps = first || moved;
        self.focused.set(focused);
        self.filled.set(filled);
        let animation = if snaps { None } else { self.animation() };
        self.settle(animation.as_ref());
    }

    /// The label's transform over its resting place.
    pub(crate) fn transform(&self) -> Operand<Affine> {
        self.transform.clone()
    }

    /// The opacity of the field's text while the label floats away from it.
    pub(crate) fn opacity(&self) -> Operand<f32> {
        self.opacity.clone()
    }

    /// Whether the text shows or is still fading.
    pub(crate) fn text_visible(&self) -> bool {
        self.is_floated() || self.opacity.snapshot() > 0.0
    }
}

/// The placeholder `prompt` as the look styles it for a field in `state`
/// over `frame`: its font follows the spec and the disabled state, its
/// colour every state.
pub fn placeholder_text(
    look: &TextFieldLook,
    prompt: waterui_core::Str,
    frame: Rect,
    state: InteractionState,
) -> StyledStr {
    let label = look.get_placeholder();
    let mut styled = StyledStr::plain(prompt);
    if let Some(font) = label.get_font() {
        let state = state.difference(LIVE_STATES);
        styled = styled.font(font.target(&Inputs::at_rest(frame, state, &())));
    }
    if let Some(color) = label.get_color() {
        styled = styled.foreground(color.target(&Inputs::at_rest(frame, state, &())));
    }
    styled
}
