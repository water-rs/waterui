//! The `AppKit` picker: one type standing in front of the three controls a
//! selection menu can take — `NSPopUpButton`, `NSSegmentedControl`, or a
//! vertical `NSStackView` of radio `NSButton`s.
//!
//! # Safety
//!
//! The `unsafe` here allocates controls through their `objc2` initializers
//! — `AppKit` control APIs are main-thread only, which the `MainThreadOnly`
//! thread kinds and [`MainThreadMarker`] constructor guarantee — and sets a
//! radio button's type through `NSButton`, a documented class call.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSButton, NSButtonType, NSControl, NSControlStateValueOff, NSControlStateValueOn, NSFont,
    NSLayoutAttribute, NSPopUpButton, NSSegmentedControl, NSStackView,
    NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSArray, NSString};

use crate::geometry::Size;
use crate::picker::PickerStyle;
use crate::{ActionTarget, view};

/// Shared with every control the picker swaps between: the selection
/// closure and the font radio buttons created later must adopt.
struct Shared {
    handler: Option<Rc<dyn Fn(usize)>>,
    font: Option<Retained<NSFont>>,
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
}

impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Control").finish_non_exhaustive()
    }
}

/// The space between a label and its sibling rows is the container's
/// business; this is the gap between stacked radio buttons.
const RADIO_SPACING: f64 = 6.0;

impl Control {
    fn view(&self) -> &NSView {
        match self {
            Self::Menu(popup) => popup,
            Self::Segmented(segmented) => segmented,
            Self::Radio(stack) => stack,
        }
    }

    /// The `NSControl` face of the single-control styles — the stack style
    /// has no control to enable or name; its buttons carry that.
    fn control(&self) -> Option<&NSControl> {
        match self {
            Self::Menu(popup) => Some(popup),
            Self::Segmented(segmented) => Some(segmented),
            Self::Radio(_) => None,
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
        };
        Self {
            control,
            targets: RefCell::new(Vec::new()),
            shared: Rc::new(RefCell::new(Shared {
                handler: None,
                font: None,
            })),
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
            Control::Radio(_) => {}
        }
    }

    /// The typeface the control draws its items in — applied to the popup
    /// or segmented control and adopted by radio buttons created later.
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

    /// The size a measure pass reports — the control's intrinsic size, or
    /// the stack's fitted size.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        let size = match &self.control {
            Control::Radio(stack) => stack.fittingSize(),
            control => control.view().intrinsicContentSize(),
        };
        Size::new(size.width, size.height)
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
