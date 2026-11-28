//! The `AppKit` picker: one type standing in front of the four controls a
//! selection menu can take — `NSPopUpButton`, `NSSegmentedControl`, a
//! vertical `NSStackView` of radio `NSButton`s, or a clipped vertical wheel
//! drum built from `AppKit` views.
//!
//! # Safety
//!
//! The `unsafe` here allocates controls through their `objc2` initializers
//! — `AppKit` control APIs are main-thread only, which the `MainThreadOnly`
//! thread kinds and [`MainThreadMarker`] constructor guarantee — sets a
//! radio button's type through `NSButton`, and defines the `NSView` subclass
//! driving the wheel drum.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use keyboard_types::{Key, NamedKey};
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityListRole, NSButton, NSButtonType, NSColor, NSControl,
    NSControlSize, NSControlStateValueOff, NSControlStateValueOn, NSEvent, NSFont,
    NSLayoutAttribute, NSPopUpButton, NSResponder, NSSegmentDistribution, NSSegmentedControl,
    NSStackView, NSTextAlignment, NSTextField, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSString};

use crate::callback::guarded;
use crate::geometry::Size;
use crate::keys::key_event;
use crate::picker::PickerStyle;
use crate::slider::ControlSize;
use crate::{ActionTarget, view};

/// Number of visible rows in the `AppKit` wheel drum (odd so one row centers).
const WHEEL_VISIBLE_ROWS: f64 = 5.0;
/// Horizontal padding inside the `AppKit` wheel drum.
const WHEEL_HORIZONTAL_PADDING: f64 = 16.0;
/// Horizontal inset of the centered selection band inside the drum.
const WHEEL_HIGHLIGHT_INSET: f64 = 6.0;
/// Minimum width of the `AppKit` wheel drum.
const WHEEL_MIN_WIDTH: f64 = 128.0;

const fn wheel_row_height(size: NSControlSize) -> f64 {
    match size {
        NSControlSize::Mini => 22.0,
        NSControlSize::Small => 26.0,
        NSControlSize::Large => 34.0,
        _ => 30.0,
    }
}

fn wheel_row_alpha(distance: f64) -> f64 {
    if distance <= 0.15 {
        return 1.0;
    }
    let half = WHEEL_VISIBLE_ROWS * 0.5;
    let factor = (1.0 - distance / (half + 0.5)).clamp(0.0, 1.0);
    (factor * 0.85).clamp(0.0, 0.72)
}

/// Colors a wheel drum paints its rows, highlight band, and container with.
struct WheelColors {
    foreground: Retained<NSColor>,
    muted: Retained<NSColor>,
    highlight: Retained<NSColor>,
    border: Retained<NSColor>,
}

impl Default for WheelColors {
    fn default() -> Self {
        Self {
            foreground: NSColor::labelColor(),
            muted: NSColor::secondaryLabelColor(),
            highlight: NSColor::unemphasizedSelectedContentBackgroundColor(),
            border: NSColor::separatorColor(),
        }
    }
}

struct WheelViewIvars {
    shared: Rc<RefCell<Shared>>,
    highlight_band: RefCell<Option<Retained<NSView>>>,
    rows: RefCell<Vec<Retained<NSTextField>>>,
    titles: RefCell<Vec<String>>,
    selected: Cell<Option<usize>>,
    enabled: Cell<bool>,
    scroll_accum: Cell<f64>,
    colors: RefCell<WheelColors>,
}

impl fmt::Debug for WheelViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WheelViewIvars")
            .field("selected", &self.selected.get())
            .field("enabled", &self.enabled.get())
            .finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSView`'s designated initializer is `initWithFrame:`, which
    // `WheelView::new` calls via `super`, and `Drop` is not implemented.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[name = "CocoaUiWheelPickerView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = WheelViewIvars]
    #[derive(Debug)]
    struct WheelView;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSView` subclass.
    unsafe impl NSObjectProtocol for WheelView {}

    impl WheelView {
        // SAFETY: flipped coordinates put `y = 0` at the top so row geometry
        // matches top-to-bottom item order directly.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        // SAFETY: `acceptsFirstResponder` returns whether the wheel can receive
        // keyboard focus.
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            self.ivars().enabled.get()
        }

        // SAFETY: updates the highlight band and row frames when bounds change.
        #[unsafe(method(layout))]
        fn layout_override(&self) {
            guarded("WheelView layout", || {
                // SAFETY: forwarding to `super` completes `NSView` layout.
                let _: () = unsafe { msg_send![super(self), layout] };
                self.layout_rows();
            });
        }

        // SAFETY: clicking a row selects it and focuses the drum for keyboard navigation.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            guarded("WheelView mouseDown:", || {
                if !self.ivars().enabled.get() {
                    return;
                }
                if let Some(window) = self.window() {
                    let _ = window.makeFirstResponder(Some(self));
                }
                let point = self.convertPoint_fromView(event.locationInWindow(), None);
                self.select_at_y(point.y);
            });
        }

        // SAFETY: scrolling vertically steps the wheel selection by row pitch.
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            guarded("WheelView scrollWheel:", || {
                if !self.ivars().enabled.get() {
                    return;
                }
                let row_h = self.row_height();
                let delta = if event.hasPreciseScrollingDeltas() {
                    event.scrollingDeltaY()
                } else {
                    event.scrollingDeltaY() * row_h
                };
                // Natural scrolling: positive deltaY pulls earlier items down;
                // negative deltaY advances to later items.
                let accum = self.ivars().scroll_accum.get() - delta;
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "bounded scroll steps across picker items"
                )]
                let steps = (accum / row_h).trunc() as isize;
                if steps == 0 {
                    self.ivars().scroll_accum.set(accum);
                    return;
                }
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "bounded scroll steps fit cleanly in f64"
                )]
                let remainder = (steps as f64).mul_add(-row_h, accum);
                self.ivars().scroll_accum.set(remainder);
                self.step_selection(steps);
            });
        }

        // SAFETY: handles Up/Down/Home/End navigation; forwards other keys to super.
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let handled = guarded("WheelView keyDown:", || {
                if !self.ivars().enabled.get() {
                    return false;
                }
                match key_event(event).key {
                    Key::Named(NamedKey::ArrowUp) => {
                        self.step_selection(-1);
                        true
                    }
                    Key::Named(NamedKey::ArrowDown) => {
                        self.step_selection(1);
                        true
                    }
                    Key::Named(NamedKey::Home) => {
                        self.commit_index(0);
                        true
                    }
                    Key::Named(NamedKey::End) => {
                        let len = self.ivars().titles.borrow().len();
                        if len > 0 {
                            self.commit_index(len - 1);
                        }
                        true
                    }
                    _ => false,
                }
            });
            if !handled {
                // SAFETY: forwarding unhandled key events preserves responder chain semantics.
                unsafe { msg_send![super(self), keyDown: event] }
            }
        }
    }
);

impl WheelView {
    fn new(mtm: MainThreadMarker, shared: Rc<RefCell<Shared>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WheelViewIvars {
            shared,
            highlight_band: RefCell::new(None),
            rows: RefCell::new(Vec::new()),
            titles: RefCell::new(Vec::new()),
            selected: Cell::new(None),
            enabled: Cell::new(true),
            scroll_accum: Cell::new(0.0),
            colors: RefCell::new(WheelColors::default()),
        });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setWantsLayer(true);
        if let Some(layer) = this.layer() {
            layer.setMasksToBounds(true);
            layer.setCornerRadius(8.0);
            layer.setBorderWidth(1.0);
        }

        let band = NSView::alloc(mtm);
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        let band: Retained<NSView> = unsafe { msg_send![band, initWithFrame: CGRect::ZERO] };
        band.setWantsLayer(true);
        if let Some(layer) = band.layer() {
            layer.setCornerRadius(6.0);
        }
        this.addSubview(&band);
        *this.ivars().highlight_band.borrow_mut() = Some(band);

        this.setAccessibilityElement(true);
        // SAFETY: `NSAccessibilityListRole` is a published AppKit extern static.
        this.setAccessibilityRole(Some(unsafe { NSAccessibilityListRole }));
        this.apply_colors();
        this
    }

    fn row_height(&self) -> f64 {
        wheel_row_height(self.ivars().shared.borrow().size)
    }

    fn set_items(&self, titles: &[String]) {
        let mtm = self.mtm();
        for row in self.ivars().rows.borrow_mut().drain(..) {
            view::remove_from_superview(&row);
        }
        *self.ivars().titles.borrow_mut() = titles.to_vec();
        let font = self.ivars().shared.borrow().font.clone();
        let mut rows = Vec::with_capacity(titles.len());
        for title in titles {
            let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
            label.setAlignment(NSTextAlignment::Center);
            label.setDrawsBackground(false);
            label.setBordered(false);
            if let Some(font) = &font {
                label.setFont(Some(font));
            }
            self.addSubview(&label);
            rows.push(label);
        }
        *self.ivars().rows.borrow_mut() = rows;
        self.update_accessibility_value();
        self.layout_rows();
        self.invalidateIntrinsicContentSize();
    }

    fn set_selected_index(&self, index: Option<usize>) {
        self.ivars().selected.set(index);
        self.update_accessibility_value();
        self.layout_rows();
    }

    fn selected_index(&self) -> Option<usize> {
        self.ivars().selected.get()
    }

    fn set_font(&self, font: &NSFont) {
        for row in self.ivars().rows.borrow().iter() {
            row.setFont(Some(font));
        }
        self.layout_rows();
        self.invalidateIntrinsicContentSize();
    }

    fn set_control_size(&self) {
        self.layout_rows();
        self.invalidateIntrinsicContentSize();
    }

    fn set_enabled(&self, enabled: bool) {
        self.ivars().enabled.set(enabled);
        self.setAlphaValue(if enabled { 1.0 } else { 0.5 });
    }

    fn set_colors(
        &self,
        foreground: &NSColor,
        muted: &NSColor,
        highlight: &NSColor,
        border: &NSColor,
    ) {
        *self.ivars().colors.borrow_mut() = WheelColors {
            foreground: foreground.into(),
            muted: muted.into(),
            highlight: highlight.into(),
            border: border.into(),
        };
        self.apply_colors();
        self.layout_rows();
    }

    fn apply_colors(&self) {
        let colors = self.ivars().colors.borrow();
        if let Some(layer) = self.layer() {
            let border_cg = colors.border.CGColor();
            layer.setBorderColor(Some(&border_cg));
        }
        if let Some(band) = self.ivars().highlight_band.borrow().as_ref()
            && let Some(layer) = band.layer()
        {
            let bg_cg = colors.highlight.CGColor();
            layer.setBackgroundColor(Some(&bg_cg));
        }
    }

    fn update_accessibility_value(&self) {
        let value = self
            .ivars()
            .selected
            .get()
            .and_then(|idx| self.ivars().titles.borrow().get(idx).cloned());
        if let Some(value) = value {
            let ns = NSString::from_str(&value);
            // SAFETY: `setAccessibilityValue:` accepts an `NSString` value object.
            unsafe { self.setAccessibilityValue(Some(AsRef::<AnyObject>::as_ref(&*ns))) };
        } else {
            // SAFETY: clearing accessibility value with `None` is documented on `NSAccessibility`.
            unsafe { self.setAccessibilityValue(None) };
        }
    }

    fn commit_index(&self, target: usize) {
        let len = self.ivars().titles.borrow().len();
        if target >= len {
            return;
        }
        if self.ivars().selected.get() == Some(target) {
            return;
        }
        self.ivars().selected.set(Some(target));
        self.update_accessibility_value();
        self.layout_rows();
        if let Some(handler) = self.ivars().shared.borrow().handler.clone() {
            handler(target);
        }
    }

    fn step_selection(&self, delta: isize) {
        let len = self.ivars().titles.borrow().len();
        if len == 0 {
            return;
        }
        let current = self.ivars().selected.get().unwrap_or(0);
        #[expect(
            clippy::cast_possible_wrap,
            clippy::cast_sign_loss,
            reason = "clamped within valid picker item bounds"
        )]
        let target = (current as isize + delta).clamp(0, (len - 1) as isize) as usize;
        self.commit_index(target);
    }

    fn select_at_y(&self, y: f64) {
        let len = self.ivars().titles.borrow().len();
        if len == 0 {
            return;
        }
        let center_y = self.bounds().size.height * 0.5;
        let row_h = self.row_height();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "bounded click offset across picker rows"
        )]
        let delta = ((y - center_y) / row_h).round() as isize;
        self.step_selection(delta);
    }

    fn layout_rows(&self) {
        let bounds = self.bounds();
        let width = bounds.size.width;
        let height = bounds.size.height;
        let row_h = self.row_height();
        let center_y = height * 0.5;
        let selected = self.ivars().selected.get().unwrap_or(0);

        if let Some(band) = self.ivars().highlight_band.borrow().as_ref() {
            let band_w = WHEEL_HIGHLIGHT_INSET.mul_add(-2.0, width).max(0.0);
            band.setFrame(CGRect::new(
                CGPoint::new(WHEEL_HIGHLIGHT_INSET, row_h.mul_add(-0.5, center_y)),
                CGSize::new(band_w, row_h),
            ));
        }

        let colors = self.ivars().colors.borrow();
        let half_visible = WHEEL_VISIBLE_ROWS * 0.5;
        for (index, row) in self.ivars().rows.borrow().iter().enumerate() {
            #[expect(
                clippy::cast_precision_loss,
                reason = "picker row indices fit well within f64 precision"
            )]
            let rel = (index as f64) - (selected as f64);
            let dist = rel.abs();
            if dist > half_visible + 0.5 {
                row.setHidden(true);
                continue;
            }
            row.setHidden(false);
            let text_h = row.fittingSize().height.min(row_h);
            let row_center = rel.mul_add(row_h, center_y);
            let row_w = WHEEL_HORIZONTAL_PADDING.mul_add(-2.0, width).max(0.0);
            row.setFrame(CGRect::new(
                CGPoint::new(WHEEL_HORIZONTAL_PADDING, text_h.mul_add(-0.5, row_center)),
                CGSize::new(row_w, text_h),
            ));
            row.setAlphaValue(wheel_row_alpha(dist));
            row.setTextColor(Some(if index == selected {
                &colors.foreground
            } else {
                &colors.muted
            }));
        }
    }

    fn intrinsic_size(&self) -> Size {
        let max_row_width = self
            .ivars()
            .rows
            .borrow()
            .iter()
            .map(|row| row.fittingSize().width)
            .fold(0.0_f64, f64::max);
        let width = WHEEL_HORIZONTAL_PADDING
            .mul_add(2.0, max_row_width)
            .max(WHEEL_MIN_WIDTH);
        let height = self.row_height() * WHEEL_VISIBLE_ROWS;
        Size::new(width, height)
    }
}

/// Shared with every control the picker swaps between: the selection
/// closure, and the font and control size radio buttons created later must
/// adopt.
struct Shared {
    handler: Option<Rc<dyn Fn(usize)>>,
    font: Option<Retained<NSFont>>,
    size: NSControlSize,
}

/// A selection menu over one of `AppKit`'s three selection controls.
///
/// The wrapped control is chosen by [`PickerStyle`] at construction and
/// stays fixed; [`Picker::view`] is what a container adds and frames.
pub struct Picker {
    control: Control,
    /// The held action targets: the control-level one for the menu and
    /// segment styles, one per arranged button for the radio style —
    /// replaced wholesale on every `set_items`.
    targets: RefCell<Vec<ActionTarget>>,
    shared: Rc<RefCell<Shared>>,
}

impl fmt::Debug for Picker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Picker")
            .field("control", &self.control)
            .finish_non_exhaustive()
    }
}

enum Control {
    Menu(Retained<NSPopUpButton>),
    Segmented(Retained<NSSegmentedControl>),
    Radio(Retained<NSStackView>),
    Wheel(Retained<WheelView>),
}

impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Control").finish_non_exhaustive()
    }
}

/// The space between a label and its sibling rows is the container's
/// business; this is the gap between stacked radio buttons — measured
/// off `SwiftUI`'s `.radioGroup` row pitch (nightly run 42559205399:
/// 6.0 drifted −1 px every second row).
const RADIO_SPACING: f64 = 6.5;

impl Control {
    fn view(&self) -> &NSView {
        match self {
            Self::Menu(popup) => popup,
            Self::Segmented(segmented) => segmented,
            Self::Radio(stack) => stack,
            Self::Wheel(wheel) => wheel,
        }
    }

    /// The `NSControl` face of the single-control styles — the stack and wheel
    /// styles have no single `NSControl` to enable or name.
    fn control(&self) -> Option<&NSControl> {
        match self {
            Self::Menu(popup) => Some(popup),
            Self::Segmented(segmented) => Some(segmented),
            Self::Radio(_) | Self::Wheel(_) => None,
        }
    }
}

impl Picker {
    /// A picker rendering `style`.
    ///
    /// # Panics
    ///
    /// Never in practice: `initWithFrame:` accepts any rectangle, including
    /// the zero rect the views start at.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, style: PickerStyle) -> Self {
        let shared = Rc::new(RefCell::new(Shared {
            handler: None,
            font: None,
            size: NSControlSize::Regular,
        }));
        let control = match style {
            PickerStyle::Menu => {
                let popup = NSPopUpButton::alloc(mtm);
                // SAFETY: `initWithFrame:pullsDown:` is `NSPopUpButton`'s
                // designated initializer.
                let popup: Retained<NSPopUpButton> =
                    unsafe { msg_send![popup, initWithFrame: CGRect::ZERO, pullsDown: false] };
                popup.setAutoenablesItems(false);
                Control::Menu(popup)
            }
            PickerStyle::Segmented => {
                let segmented = NSSegmentedControl::alloc(mtm);
                // SAFETY: `initWithFrame:` is the inherited designated
                // initializer.
                let segmented: Retained<NSSegmentedControl> =
                    unsafe { msg_send![segmented, initWithFrame: CGRect::ZERO] };
                // The row fills the width it is given, dividing it into
                // equal segments (`docs/layout-spec.md` §6).
                segmented.setSegmentDistribution(NSSegmentDistribution::FillEqually);
                Control::Segmented(segmented)
            }
            PickerStyle::Radio => {
                let stack = NSStackView::alloc(mtm);
                // SAFETY: `initWithFrame:` is the inherited designated
                // initializer.
                let stack: Retained<NSStackView> =
                    unsafe { msg_send![stack, initWithFrame: CGRect::ZERO] };
                stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
                stack.setAlignment(NSLayoutAttribute::Leading);
                stack.setSpacing(RADIO_SPACING);
                Control::Radio(stack)
            }
            PickerStyle::Wheel => Control::Wheel(WheelView::new(mtm, Rc::clone(&shared))),
        };
        Self {
            control,
            targets: RefCell::new(Vec::new()),
            shared,
        }
    }

    /// The view the picker renders — one of its wrapped controls. Add it to
    /// a container and frame it like any other view.
    #[must_use]
    pub fn view(&self) -> &NSView {
        self.control.view()
    }

    /// Replaces the items the control offers, keeping the current
    /// selection's index when it still names a row.
    pub fn set_items(&self, titles: &[String]) {
        match &self.control {
            Control::Menu(popup) => {
                popup.removeAllItems();
                let titles: Vec<Retained<NSString>> = titles
                    .iter()
                    .map(|title| NSString::from_str(title))
                    .collect();
                popup.addItemsWithTitles(&NSArray::from_retained_slice(&titles));
            }
            Control::Segmented(segmented) => {
                segmented.setSegmentCount(titles.len().cast_signed());
                for (index, title) in titles.iter().enumerate() {
                    segmented.setLabel_forSegment(&NSString::from_str(title), index.cast_signed());
                }
            }
            Control::Radio(stack) => {
                for arranged in &stack.arrangedSubviews() {
                    stack.removeArrangedSubview(&arranged);
                    view::remove_from_superview(&arranged);
                }
                self.targets.borrow_mut().clear();
                for (index, title) in titles.iter().enumerate() {
                    // SAFETY: `radioButtonWithTitle:target:action:` builds
                    // a standard radio button; `None` target and action
                    // leave it inert until the `ActionTarget` lands below.
                    let button = unsafe {
                        NSButton::radioButtonWithTitle_target_action(
                            &NSString::from_str(title),
                            None,
                            None,
                            MainThreadMarker::from(self.view()),
                        )
                    };
                    button.setButtonType(NSButtonType::Radio);
                    button.setControlSize(self.shared.borrow().size);
                    if let Some(font) = &self.shared.borrow().font {
                        button.setFont(Some(font));
                    }
                    let shared = Rc::clone(&self.shared);
                    let control: &NSControl = &button;
                    self.targets
                        .borrow_mut()
                        .push(ActionTarget::new(control, move |_mtm| {
                            if let Some(handler) = &shared.borrow().handler {
                                handler(index);
                            }
                        }));
                    stack.addArrangedSubview(&button);
                }
            }
            Control::Wheel(wheel) => wheel.set_items(titles),
        }
    }

    /// Selects the item at `index`; `None` clears the selection (radio
    /// buttons all go dark, the menu keeps its title row but no checked
    /// item, segments deselect).
    pub fn set_selected_index(&self, index: Option<usize>) {
        match &self.control {
            Control::Menu(popup) => match index {
                Some(index) => popup.selectItemAtIndex(index.cast_signed()),
                None => popup.selectItem(None),
            },
            Control::Segmented(segmented) => {
                segmented.setSelectedSegment(index.map_or(-1, usize::cast_signed));
            }
            Control::Radio(stack) => {
                for (i, arranged) in stack.arrangedSubviews().iter().enumerate() {
                    let Ok(button) = arranged.downcast::<NSButton>() else {
                        continue;
                    };
                    button.setState(if Some(i) == index {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                }
            }
            Control::Wheel(wheel) => wheel.set_selected_index(index),
        }
    }

    /// The selected index, when one is — mirrors `set_selected_index`.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        match &self.control {
            Control::Menu(popup) => usize::try_from(popup.indexOfSelectedItem()).ok(),
            Control::Segmented(segmented) => usize::try_from(segmented.selectedSegment()).ok(),
            Control::Radio(stack) => {
                stack
                    .arrangedSubviews()
                    .iter()
                    .enumerate()
                    .find_map(|(i, arranged)| {
                        let button = arranged.downcast::<NSButton>().ok()?;
                        (button.state() == NSControlStateValueOn).then_some(i)
                    })
            }
            Control::Wheel(wheel) => wheel.selected_index(),
        }
    }

    /// Sets the handler called with the chosen index each time the user
    /// picks an item — once; the picker holds its targets.
    pub fn install_action(&self, handler: impl Fn(usize) + 'static) {
        self.shared.borrow_mut().handler = Some(Rc::new(handler));
        match &self.control {
            Control::Menu(popup) => {
                let shared = Rc::clone(&self.shared);
                let this: Weak<NSPopUpButton> = Weak::new(popup);
                self.targets
                    .borrow_mut()
                    .push(ActionTarget::new(popup, move |_mtm| {
                        if let Some(popup) = this.load()
                            && let Some(handler) = &shared.borrow().handler
                            && let Ok(index) = usize::try_from(popup.indexOfSelectedItem())
                        {
                            handler(index);
                        }
                    }));
            }
            Control::Segmented(segmented) => {
                let shared = Rc::clone(&self.shared);
                let this: Weak<NSSegmentedControl> = Weak::new(segmented);
                self.targets
                    .borrow_mut()
                    .push(ActionTarget::new(segmented, move |_mtm| {
                        if let Some(segmented) = this.load()
                            && let Some(handler) = &shared.borrow().handler
                            && let Ok(index) = usize::try_from(segmented.selectedSegment())
                        {
                            handler(index);
                        }
                    }));
            }
            Control::Radio(_) | Control::Wheel(_) => {}
        }
    }

    /// The typeface the control draws its items in — applied to the popup,
    /// segmented control, wheel drum, and adopted by radio buttons created
    /// later.
    pub fn set_font(&self, font: &NSFont) {
        self.shared.borrow_mut().font = Some(font.into());
        match &self.control {
            Control::Menu(popup) => popup.setFont(Some(font)),
            Control::Segmented(segmented) => segmented.setFont(Some(font)),
            Control::Radio(stack) => {
                for arranged in &stack.arrangedSubviews() {
                    if let Ok(button) = arranged.downcast::<NSButton>() {
                        button.setFont(Some(font));
                    }
                }
            }
            Control::Wheel(wheel) => wheel.set_font(font),
        }
    }

    /// How large the control is drawn: `NSControl.controlSize` on the menu
    /// or the segmented control, and on every radio button, those created
    /// later included.
    pub fn set_control_size(&self, size: ControlSize) {
        let size = match size {
            ControlSize::Mini => NSControlSize::Mini,
            ControlSize::Small => NSControlSize::Small,
            ControlSize::Regular => NSControlSize::Regular,
            ControlSize::Large => NSControlSize::Large,
        };
        self.shared.borrow_mut().size = size;
        match &self.control {
            Control::Radio(stack) => {
                for arranged in &stack.arrangedSubviews() {
                    if let Ok(button) = arranged.downcast::<NSButton>() {
                        button.setControlSize(size);
                    }
                }
            }
            Control::Wheel(wheel) => wheel.set_control_size(),
            _ => {
                if let Some(control) = self.control.control() {
                    control.setControlSize(size);
                }
            }
        }
    }

    /// Applies semantic theme colours to the wheel drum; no-op for other styles.
    pub fn set_wheel_colors(
        &self,
        foreground: &NSColor,
        muted: &NSColor,
        highlight: &NSColor,
        border: &NSColor,
    ) {
        if let Control::Wheel(wheel) = &self.control {
            wheel.set_colors(foreground, muted, highlight, border);
        }
    }

    /// Whether the control responds to input — for the stack style, every
    /// arranged button.
    pub fn set_enabled(&self, enabled: bool) {
        match &self.control {
            Control::Radio(stack) => {
                for arranged in &stack.arrangedSubviews() {
                    if let Ok(button) = arranged.downcast::<NSButton>() {
                        button.setEnabled(enabled);
                    }
                }
            }
            Control::Wheel(wheel) => wheel.set_enabled(enabled),
            _ => {
                if let Some(control) = self.control.control() {
                    control.setEnabled(enabled);
                }
            }
        }
    }

    /// The height of the first radio row — the vertical anchor a leading
    /// label centers on for the stack style; `None` on other styles or an
    /// empty stack.
    #[must_use]
    pub fn first_row_height(&self) -> Option<f64> {
        if let Control::Radio(stack) = &self.control {
            stack
                .arrangedSubviews()
                .iter()
                .next()
                .map(|view| view.fittingSize().height)
        } else {
            None
        }
    }

    /// The size a measure pass reports — the control's intrinsic size, the
    /// stack's fitted size, or the wheel drum's intrinsic size.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        match &self.control {
            Control::Radio(stack) => {
                let size = stack.fittingSize();
                Size::new(size.width, size.height)
            }
            Control::Wheel(wheel) => wheel.intrinsic_size(),
            control => {
                let size = control.view().intrinsicContentSize();
                Size::new(size.width, size.height)
            }
        }
    }

    /// Names the control to a screen reader; `None` leaves it unnamed.
    /// For the stack style the stack itself takes the name — its buttons
    /// keep speaking their item titles.
    pub fn set_accessibility_label(&self, label: Option<&str>) {
        if let Some(label) = label {
            view::set_accessibility_label(self.view(), label);
        }
    }
}
