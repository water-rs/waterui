//! The `UIKit` picker: one type standing in front of the three controls a
//! selection menu can take — a `UIButton` presenting a `UIMenu`, a
//! `UISegmentedControl`, or a single-column `UIPickerView` wheel standing
//! in for the radio group `UIKit` does not have.
//!
//! # Safety
//!
//! The `unsafe` here allocates controls through their `objc2` initializers,
//! creates `UIAction`s, and builds the wheel's delegate object — `UIKit`
//! APIs are main-thread only, which the `MainThreadOnly` thread kinds and
//! [`MainThreadMarker`] constructor guarantee.

use std::cell::RefCell;
use std::fmt;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::{Allocated, Retained, Weak};
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_foundation::{
    NSArray, NSAttributedString, NSMutableAttributedString, NSObject, NSObjectProtocol, NSRange,
    NSString,
};
use objc2_ui_kit::{
    NSDirectionalRectEdge, NSFontAttributeName, NSTextAlignment, UIAction, UIButton,
    UIButtonConfiguration, UIControlState, UIFont, UIImage, UIImageSymbolConfiguration,
    UIImageSymbolScale, UILabel, UIMenu, UIMenuElement, UIMenuElementState, UIPickerView,
    UIPickerViewDataSource, UIPickerViewDelegate, UISegmentedControl, UIView,
};

use crate::ActionTarget;
use crate::action::ControlEvents;
use crate::geometry::Size;
use crate::picker::PickerStyle;

/// The menu button's trailing chevron and its padding — the symbol the
/// platform's own menu controls draw.
const CHEVRON: &str = "chevron.up.chevron.down";
const CHEVRON_PADDING: f64 = 4.0;

/// The point size a wheel row draws at when no font is applied — the
/// default `UIFont.systemFont(ofSize: 21)` the picker uses.
const WHEEL_FONT_SIZE: f64 = 21.0;

/// Shared with the menu's `UIAction`s and the wheel's delegate: the
/// selection closure, the current titles and the font wheel rows adopt.
struct Shared {
    handler: Option<Rc<dyn Fn(usize)>>,
    titles: Vec<String>,
    font: Option<Retained<UIFont>>,
}

/// The wheel's `UIPickerViewDataSource`/`UIPickerViewDelegate`: a plain
/// `NSObject` reading the picker's shared state.
struct WheelIvars {
    shared: Rc<RefCell<Shared>>,
}

impl fmt::Debug for WheelIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WheelIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSObject`'s designated initializer is `init`, which the
    // override below calls, and the class does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiPickerWheelDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = WheelIvars]
    /// Data source and delegate of a `Picker`'s wheel column.
    struct WheelDelegate;

    impl WheelDelegate {
        // SAFETY: `init` is `NSObject`'s designated initializer.
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Retained<Self> {
            let this = this.set_ivars(WheelIvars {
                shared: Rc::new(RefCell::new(Shared {
                    handler: None,
                    titles: Vec::new(),
                    font: None,
                })),
            });
            // SAFETY: `init` is `NSObject`'s designated initializer.
            unsafe { msg_send![super(this), init] }
        }
    }

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for WheelDelegate {}

    // SAFETY: both required methods read only the shared item list.
    unsafe impl UIPickerViewDataSource for WheelDelegate {
        #[unsafe(method(numberOfComponentsInPickerView:))]
        fn number_of_components(&self, _picker_view: &UIPickerView) -> isize {
            1
        }

        #[unsafe(method(pickerView:numberOfRowsInComponent:))]
        fn number_of_rows(&self, _picker_view: &UIPickerView, _component: isize) -> isize {
            self.ivars().shared.borrow().titles.len().cast_signed()
        }
    }

    // SAFETY: the row view is built and labelled on the main thread, and
    // `didSelectRow` only forwards the row index.
    unsafe impl UIPickerViewDelegate for WheelDelegate {
        #[unsafe(method_id(pickerView:viewForRow:forComponent:reusingView:))]
        fn view_for_row(
            &self,
            _picker_view: &UIPickerView,
            row: isize,
            _component: isize,
            reusing_view: Option<&UIView>,
        ) -> Retained<UIView> {
            let label = reusing_view.and_then(|v| v.downcast_ref::<UILabel>()).map_or_else(
                || {
                    let label = UILabel::new(self.mtm());
                    label.setTextAlignment(NSTextAlignment::Center);
                    label.setAdjustsFontSizeToFitWidth(true);
                    label
                },
                Message::retain,
            );
            let shared = self.ivars().shared.borrow();
            if let Some(title) = shared.titles.get(row.cast_unsigned()) {
                label.setText(Some(&NSString::from_str(title)));
            }
            // A wheel row always draws at 21pt — the picker's own row
            // size — so the applied font is resized to it.
            let font = shared
                .font
                .as_ref()
                .map_or_else(
                    || UIFont::systemFontOfSize(WHEEL_FONT_SIZE),
                    |font| font.fontWithSize(WHEEL_FONT_SIZE),
                );
            // SAFETY: `setFont:` accepts any `UIFont` or none.
            unsafe { label.setFont(Some(&font)) };
            label.into_super()
        }

        #[unsafe(method(pickerView:didSelectRow:inComponent:))]
        fn did_select_row(&self, _picker_view: &UIPickerView, row: isize, _component: isize) {
            if let Some(handler) = &self.ivars().shared.borrow().handler {
                handler(row.cast_unsigned());
            }
        }
    }
);

impl WheelDelegate {
    /// A delegate reading `shared` — the same cell its `Picker` writes, so
    /// `number_of_rows` sees the titles `set_items` installed.
    fn with_shared(mtm: MainThreadMarker, shared: Rc<RefCell<Shared>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WheelIvars { shared });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// A selection menu over one of `UIKit`'s selection controls.
///
/// The wrapped control is chosen by [`PickerStyle`] at construction and
/// stays fixed; [`Picker::view`] is what a container adds and frames.
pub struct Picker {
    control: Control,
    mtm: MainThreadMarker,
    /// The control-level action target the segment style fires through;
    /// the menu's `UIAction`s and the wheel's delegate call the handler
    /// directly.
    targets: RefCell<Vec<ActionTarget>>,
    /// The menu style's current `UIAction` children, kept for check-state
    /// updates.
    actions: RefCell<Vec<Retained<UIAction>>>,
    /// The wheel style's delegate; `Some` only for [`PickerStyle::Radio`].
    _delegate: Option<Retained<WheelDelegate>>,
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
    Menu(Retained<UIButton>),
    Segmented(Retained<UISegmentedControl>),
    Wheel(Retained<UIPickerView>),
}

impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Control").finish_non_exhaustive()
    }
}

impl Control {
    fn view(&self) -> &UIView {
        match self {
            Self::Menu(button) => button,
            Self::Segmented(segmented) => segmented,
            Self::Wheel(wheel) => wheel,
        }
    }
}

/// The attributed title the menu button draws — the text in the picker's
/// font when one is applied, plain otherwise.
fn menu_title(
    text: &str,
    font: Option<&UIFont>,
    mtm: MainThreadMarker,
) -> Retained<NSAttributedString> {
    let title = NSMutableAttributedString::initWithString(
        mtm.alloc::<NSMutableAttributedString>(),
        &NSString::from_str(text),
    );
    if let Some(font) = font {
        let range = NSRange::new(0, title.length());
        // SAFETY: `title` is a live mutable attributed string and the
        // attribute name is a static the platform exports; the value is a
        // `UIFont`, the documented type.
        unsafe {
            title.addAttribute_value_range(NSFontAttributeName, font.as_ref(), range);
        }
    }
    title.into_super()
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
            titles: Vec::new(),
            font: None,
        }));
        let (control, delegate) = match style {
            PickerStyle::Menu => {
                let button = UIButton::new(mtm);
                let configuration = UIButtonConfiguration::plainButtonConfiguration(mtm);
                configuration
                    .setImage(UIImage::systemImageNamed(&NSString::from_str(CHEVRON)).as_deref());
                configuration.setImagePlacement(NSDirectionalRectEdge::Trailing);
                configuration.setImagePadding(CHEVRON_PADDING);
                configuration.setPreferredSymbolConfigurationForImage(Some(
                    &UIImageSymbolConfiguration::configurationWithScale(UIImageSymbolScale::Small),
                ));
                button.setConfiguration(Some(&configuration));
                button.setShowsMenuAsPrimaryAction(true);
                (Control::Menu(button), None)
            }
            PickerStyle::Segmented => {
                let segmented = UISegmentedControl::new(mtm);
                (Control::Segmented(segmented), None)
            }
            PickerStyle::Radio => {
                let wheel = UIPickerView::new(mtm);
                let delegate = WheelDelegate::with_shared(mtm, Rc::clone(&shared));
                wheel.setDataSource(Some(ProtocolObject::from_ref(&*delegate)));
                wheel.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                (Control::Wheel(wheel), Some(delegate))
            }
        };
        Self {
            control,
            mtm,
            targets: RefCell::new(Vec::new()),
            actions: RefCell::new(Vec::new()),
            _delegate: delegate,
            shared,
        }
    }

    /// The view the picker renders — one of its wrapped controls. Add it to
    /// a container and frame it like any other view.
    #[must_use]
    pub fn view(&self) -> &UIView {
        self.control.view()
    }

    /// Replaces the items the control offers. The menu style rebuilds its
    /// `UIMenu`; the wheel reloads its column.
    pub fn set_items(&self, titles: &[String]) {
        self.shared.borrow_mut().titles = titles.to_vec();
        match &self.control {
            Control::Menu(button) => {
                let mut actions = self.actions.borrow_mut();
                actions.clear();
                for (index, title) in titles.iter().enumerate() {
                    let shared = Rc::clone(&self.shared);
                    let block = RcBlock::new(move |_action: NonNull<UIAction>| {
                        if let Some(handler) = &shared.borrow().handler {
                            handler(index);
                        }
                    });
                    // SAFETY: `handler` takes a block the call copies, so
                    // the stack block may be dropped once the call returns.
                    let action = unsafe {
                        UIAction::actionWithTitle_image_identifier_handler(
                            &NSString::from_str(title),
                            None,
                            None,
                            RcBlock::as_ptr(&block).cast(),
                            self.mtm,
                        )
                    };
                    actions.push(action);
                }
                let elements: Vec<Retained<UIMenuElement>> = actions
                    .iter()
                    .map(|action| action.clone().into_super())
                    .collect();
                let menu =
                    UIMenu::menuWithChildren(&NSArray::from_retained_slice(&elements), self.mtm);
                button.setMenu(Some(&menu));
            }
            Control::Segmented(segmented) => {
                segmented.removeAllSegments();
                for (index, title) in titles.iter().enumerate() {
                    segmented.insertSegmentWithTitle_atIndex_animated(
                        Some(&NSString::from_str(title)),
                        index,
                        false,
                    );
                }
            }
            Control::Wheel(wheel) => {
                wheel.reloadAllComponents();
            }
        }
    }

    /// Selects the item at `index`; `None` clears the selection.
    pub fn set_selected_index(&self, index: Option<usize>) {
        match &self.control {
            Control::Menu(button) => {
                for (i, action) in self.actions.borrow().iter().enumerate() {
                    action.setState(if Some(i) == index {
                        UIMenuElementState::On
                    } else {
                        UIMenuElementState::Off
                    });
                }
                let shared = self.shared.borrow();
                let title = index
                    .and_then(|i| shared.titles.get(i))
                    .map_or("", String::as_str);
                button.setAttributedTitle_forState(
                    Some(&menu_title(title, shared.font.as_deref(), self.mtm)),
                    UIControlState::Normal,
                );
            }
            Control::Segmented(segmented) => {
                segmented.setSelectedSegmentIndex(index.map_or(-1, usize::cast_signed));
            }
            Control::Wheel(wheel) => {
                wheel.selectRow_inComponent_animated(
                    index.map_or(-1, usize::cast_signed),
                    0,
                    false,
                );
            }
        }
    }

    /// The selected index, when one is — mirrors `set_selected_index`.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        match &self.control {
            Control::Menu(_) => self
                .actions
                .borrow()
                .iter()
                .enumerate()
                .find_map(|(i, action)| (action.state() == UIMenuElementState::On).then_some(i)),
            Control::Segmented(segmented) => usize::try_from(segmented.selectedSegmentIndex()).ok(),
            Control::Wheel(wheel) => usize::try_from(wheel.selectedRowInComponent(0)).ok(),
        }
    }

    /// Sets the handler called with the chosen index each time the user
    /// picks an item — once; the picker holds its targets.
    pub fn install_action(&self, handler: impl Fn(usize) + 'static) {
        let handler: Rc<dyn Fn(usize)> = Rc::new(handler);
        self.shared.borrow_mut().handler = Some(Rc::clone(&handler));
        if let Control::Segmented(segmented) = &self.control {
            let this: Weak<UISegmentedControl> = Weak::new(segmented);
            self.targets.borrow_mut().push(ActionTarget::new(
                segmented,
                ControlEvents::VALUE_CHANGED,
                move |_mtm| {
                    if let Some(segmented) = this.load()
                        && let Ok(index) = usize::try_from(segmented.selectedSegmentIndex())
                    {
                        handler(index);
                    }
                },
            ));
        }
    }

    /// The typeface the control draws its items in — the wheel's row
    /// labels and the menu button's title adopt it.
    pub fn set_font(&self, font: &UIFont) {
        self.shared.borrow_mut().font = Some(font.into());
        match &self.control {
            Control::Menu(button) => {
                // Refresh the attributed title so the new face applies.
                let shared = self.shared.borrow();
                let index = self.selected_index();
                let title = index
                    .and_then(|i| shared.titles.get(i))
                    .map_or("", String::as_str);
                button.setAttributedTitle_forState(
                    Some(&menu_title(title, Some(font), self.mtm)),
                    UIControlState::Normal,
                );
            }
            Control::Wheel(wheel) => {
                wheel.reloadAllComponents();
            }
            Control::Segmented(segmented) => {
                // SAFETY: `NSFontAttributeName` is a static the platform
                // exports; reading it is a constant load.
                let attributes = objc2_foundation::NSDictionary::from_slices(
                    &[unsafe { NSFontAttributeName }],
                    &[font.as_ref()],
                );
                // SAFETY: `setTitleTextAttributes:forState:` takes an
                // attribute dictionary; the name is the static the platform
                // exports and the value a `UIFont`.
                unsafe {
                    segmented
                        .setTitleTextAttributes_forState(Some(&attributes), UIControlState::Normal);
                }
            }
        }
    }

    /// Whether the control responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        match &self.control {
            Control::Menu(button) => button.setEnabled(enabled),
            Control::Segmented(segmented) => segmented.setEnabled(enabled),
            Control::Wheel(wheel) => wheel.setUserInteractionEnabled(enabled),
        }
    }

    /// The size a measure pass reports — the control's intrinsic size.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        let size = self.view().intrinsicContentSize();
        Size::new(size.width, size.height)
    }
}
