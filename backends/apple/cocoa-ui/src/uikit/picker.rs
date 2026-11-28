//! The `UIKit` picker: one type standing in front of the four controls a
//! selection menu can take — a `UIButton` presenting a `UIMenu`, a
//! `UISegmentedControl`, a vertical radio-button group composed of native
//! `UIControl` rows, or a single-column `UIPickerView` wheel.
//!
//! # Safety
//!
//! The `unsafe` here allocates controls through their `objc2` initializers,
//! creates `UIAction`s, defines `UIView`/`UIControl` subclasses for the radio
//! group, and builds the wheel's delegate object — `UIKit` APIs are
//! main-thread only, which the `MainThreadOnly` thread kinds and
//! [`MainThreadMarker`] constructor guarantee.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::{Allocated, Retained, Weak};
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{
    NSArray, NSAttributedString, NSMutableAttributedString, NSObject, NSObjectProtocol, NSRange,
    NSString, ns_string,
};
use objc2_ui_kit::{
    NSDirectionalRectEdge, NSFontAttributeName, NSObjectUIAccessibility, NSTextAlignment,
    UIAccessibilityTraitButton, UIAccessibilityTraitNotEnabled, UIAccessibilityTraitSelected,
    UIAction, UIButton, UIButtonConfiguration, UIColor, UIControl, UIControlState, UIFont,
    UIFontTextStyleBody, UIImage, UIImageRenderingMode, UIImageSymbolConfiguration,
    UIImageSymbolScale, UIImageView, UILabel, UIMenu, UIMenuElement, UIMenuElementState,
    UIPickerView, UIPickerViewDataSource, UIPickerViewDelegate, UIResponder, UISegmentedControl,
    UIUserInterfaceLayoutDirection, UIView,
};

use crate::ActionTarget;
use crate::action::ControlEvents;
use crate::callback::guarded;
use crate::geometry::Size;
use crate::picker::PickerStyle;
use crate::slider::ControlSize;
use crate::view;

/// The menu button's trailing chevron and its padding — the symbol the
/// platform's own menu controls draw.
const CHEVRON: &str = "chevron.up.chevron.down";
const CHEVRON_PADDING: f64 = 4.0;

/// Vertical gap between stacked radio rows, in points.
const RADIO_ROW_SPACING: f64 = 8.0;
/// Gap between a radio row's circle indicator and its label, in points.
const RADIO_ICON_SPACING: f64 = 10.0;
/// Side length of the native radio circle symbol, in points.
const RADIO_ICON_SIZE: f64 = 20.0;
/// Minimum height of a tappable radio row, in points.
const RADIO_MIN_ROW_HEIGHT: f64 = 32.0;

/// The point size a wheel row draws at when no font is applied — the
/// default `UIFont.systemFont(ofSize: 21)` the picker uses.
const WHEEL_FONT_SIZE: f64 = 21.0;

/// The height of one wheel row, in points.
///
/// It is the row pitch `SwiftUI`'s `.wheel`/`.inline` picker gives its
/// `UIPickerView`: its coordinator's
/// `pickerView(_:rowHeightForComponent:)` returns a constant 30 (the
/// `UIKitWheelPicker` coordinator in `SwiftUI.framework`). Without the
/// delegate method `UIKit` falls back to its style default
/// (`-[_UIPickerViewStyle_iOS defaultRowHeightForSize:]`: 32 pt for a
/// wheel at least 216 pt tall, 24 pt below), which spaces the rows 2 pt
/// further apart than the platform's own picker.
pub const WHEEL_ROW_HEIGHT: f64 = 30.0;

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

    // SAFETY: the row view is built and labelled on the main thread, the
    // row height is a constant `CGFloat` (`f64` on every supported Apple
    // target), and `didSelectRow` only forwards the row index.
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

        #[unsafe(method(pickerView:rowHeightForComponent:))]
        fn row_height(&self, _picker_view: &UIPickerView, _component: isize) -> f64 {
            WHEEL_ROW_HEIGHT
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

struct RadioColors {
    accent: Retained<UIColor>,
    foreground: Retained<UIColor>,
    muted: Retained<UIColor>,
}

impl Default for RadioColors {
    fn default() -> Self {
        Self {
            accent: UIColor::systemBlueColor(),
            foreground: UIColor::labelColor(),
            muted: UIColor::secondaryLabelColor(),
        }
    }
}

struct RadioRowIvars {
    icon: RefCell<Option<Retained<UIImageView>>>,
    title: RefCell<Option<Retained<UILabel>>>,
    selected_image: RefCell<Option<Retained<UIImage>>>,
    unselected_image: RefCell<Option<Retained<UIImage>>>,
    colors: Rc<RefCell<RadioColors>>,
}

impl fmt::Debug for RadioRowIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RadioRowIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `UIControl`'s designated initializer is `initWithFrame:`, which
    // `RadioRowView::new` calls via `super`, and `Drop` is not implemented.
    #[unsafe(super(UIControl, UIView, UIResponder, NSObject))]
    #[name = "CocoaUiRadioRowControl"]
    #[thread_kind = MainThreadOnly]
    #[ivars = RadioRowIvars]
    #[derive(Debug)]
    struct RadioRowView;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIControl` subclass.
    unsafe impl NSObjectProtocol for RadioRowView {}

    impl RadioRowView {
        // SAFETY: arranges the radio circle icon and item label inside the row bounds.
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews_override(&self) {
            guarded("RadioRowView layoutSubviews", || {
                // SAFETY: forwarding to `super` completes `UIView` layout.
                let _: () = unsafe { msg_send![super(self), layoutSubviews] };
                let bounds = self.bounds();
                let width = bounds.size.width;
                let height = bounds.size.height;
                let rtl = self.effectiveUserInterfaceLayoutDirection()
                    == UIUserInterfaceLayoutDirection::RightToLeft;
                let icon_y = (height - RADIO_ICON_SIZE) * 0.5;
                let icon_x = if rtl {
                    (width - RADIO_ICON_SIZE).max(0.0)
                } else {
                    0.0
                };
                if let Some(icon) = self.ivars().icon.borrow().as_ref() {
                    icon.setFrame(CGRect::new(
                        CGPoint::new(icon_x, icon_y),
                        CGSize::new(RADIO_ICON_SIZE, RADIO_ICON_SIZE),
                    ));
                }
                if let Some(label) = self.ivars().title.borrow().as_ref() {
                    let label_w = (width - RADIO_ICON_SIZE - RADIO_ICON_SPACING).max(0.0);
                    let label_x = if rtl {
                        0.0
                    } else {
                        RADIO_ICON_SIZE + RADIO_ICON_SPACING
                    };
                    label.setFrame(CGRect::new(
                        CGPoint::new(label_x, 0.0),
                        CGSize::new(label_w, height),
                    ));
                }
            });
        }
    }
);

impl RadioRowView {
    fn new(
        mtm: MainThreadMarker,
        title: &str,
        font: Option<&UIFont>,
        colors: Rc<RefCell<RadioColors>>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RadioRowIvars {
            icon: RefCell::new(None),
            title: RefCell::new(None),
            selected_image: RefCell::new(None),
            unselected_image: RefCell::new(None),
            colors,
        });
        // SAFETY: `initWithFrame:` is `UIControl`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };

        let unselected = UIImage::systemImageNamed(ns_string!("circle"))
            .map(|img| img.imageWithRenderingMode(UIImageRenderingMode::AlwaysTemplate));
        let selected = UIImage::systemImageNamed(ns_string!("largecircle.fill.circle"))
            .map(|img| img.imageWithRenderingMode(UIImageRenderingMode::AlwaysTemplate));

        let icon = UIImageView::new(mtm);
        // SAFETY: `UIFontTextStyleBody` is an immutable UIKit shared text style.
        let symbol_cfg =
            unsafe { UIImageSymbolConfiguration::configurationWithTextStyle(UIFontTextStyleBody) };
        icon.setPreferredSymbolConfiguration(Some(&symbol_cfg));
        icon.setImage(unselected.as_deref());
        icon.setUserInteractionEnabled(false);
        this.addSubview(&icon);
        *this.ivars().unselected_image.borrow_mut() = unselected;
        *this.ivars().selected_image.borrow_mut() = selected;
        *this.ivars().icon.borrow_mut() = Some(icon);

        let label = UILabel::new(mtm);
        label.setText(Some(&NSString::from_str(title)));
        label.setUserInteractionEnabled(false);
        if let Some(font) = font {
            // SAFETY: `setFont:` accepts any `UIFont`.
            unsafe { label.setFont(Some(font)) };
        }
        this.addSubview(&label);
        *this.ivars().title.borrow_mut() = Some(label);

        this.setIsAccessibilityElement(true, mtm);
        this.setAccessibilityLabel(Some(&NSString::from_str(title)), mtm);
        this.setAccessibilityValue(Some(ns_string!("radio button")), mtm);
        this.apply_visual_state();
        this
    }

    fn apply_visual_state(&self) {
        let mtm = self.mtm();
        let selected = self.isSelected();
        let enabled = self.isEnabled();
        {
            let colors = self.ivars().colors.borrow();
            if let Some(icon) = self.ivars().icon.borrow().as_ref() {
                let image = if selected {
                    self.ivars().selected_image.borrow().clone()
                } else {
                    self.ivars().unselected_image.borrow().clone()
                };
                icon.setImage(image.as_deref());
                // SAFETY: `setTintColor:` is marked unsafe in the bindings; it is
                // an ordinary main-thread `UIImageView` property setter.
                unsafe {
                    icon.setTintColor(Some(if selected {
                        &colors.accent
                    } else {
                        &colors.muted
                    }));
                }
            }
            if let Some(label) = self.ivars().title.borrow().as_ref() {
                // SAFETY: `setTextColor:` is marked unsafe in the bindings; it is
                // an ordinary main-thread `UILabel` property setter.
                unsafe {
                    label.setTextColor(Some(if enabled {
                        &colors.foreground
                    } else {
                        &colors.muted
                    }));
                }
            }
        }
        self.setAlpha(if enabled { 1.0 } else { 0.5 });
        // SAFETY: `UIAccessibilityTrait*` are published UIKit extern statics.
        let mut traits = unsafe { UIAccessibilityTraitButton };
        if selected {
            // SAFETY: published UIKit extern static.
            traits |= unsafe { UIAccessibilityTraitSelected };
        }
        if !enabled {
            // SAFETY: published UIKit extern static.
            traits |= unsafe { UIAccessibilityTraitNotEnabled };
        }
        self.setAccessibilityTraits(traits, mtm);
    }

    fn set_selected_row(&self, selected: bool) {
        self.setSelected(selected);
        self.apply_visual_state();
    }

    fn set_enabled_row(&self, enabled: bool) {
        self.setEnabled(enabled);
        self.apply_visual_state();
    }

    fn set_font(&self, font: &UIFont) {
        if let Some(label) = self.ivars().title.borrow().as_ref() {
            // SAFETY: `setFont:` accepts any `UIFont`.
            unsafe { label.setFont(Some(font)) };
        }
        self.setNeedsLayout();
    }

    fn intrinsic_row_size(&self) -> Size {
        let label_size = self
            .ivars()
            .title
            .borrow()
            .as_ref()
            .map_or(CGSize::ZERO, |label| {
                label.sizeThatFits(CGSize::new(f64::MAX, f64::MAX))
            });
        let width = RADIO_ICON_SIZE + RADIO_ICON_SPACING + label_size.width;
        let height = label_size
            .height
            .max(RADIO_ICON_SIZE)
            .max(RADIO_MIN_ROW_HEIGHT);
        Size::new(width, height)
    }
}

struct RadioGroupIvars {
    rows: RefCell<Vec<Retained<RadioRowView>>>,
    selected: Cell<Option<usize>>,
    enabled: Cell<bool>,
    colors: Rc<RefCell<RadioColors>>,
}

impl fmt::Debug for RadioGroupIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RadioGroupIvars")
            .field("selected", &self.selected.get())
            .field("enabled", &self.enabled.get())
            .finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `UIView`'s designated initializer is `initWithFrame:`, which
    // `RadioGroupView::new` calls via `super`, and `Drop` is not implemented.
    #[unsafe(super(UIView, UIResponder, NSObject))]
    #[name = "CocoaUiRadioGroupView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = RadioGroupIvars]
    #[derive(Debug)]
    struct RadioGroupView;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIView` subclass.
    unsafe impl NSObjectProtocol for RadioGroupView {}

    impl RadioGroupView {
        // SAFETY: lays out stacked radio rows from top to bottom inside bounds.
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews_override(&self) {
            guarded("RadioGroupView layoutSubviews", || {
                // SAFETY: forwarding to `super` completes `UIView` layout.
                let _: () = unsafe { msg_send![super(self), layoutSubviews] };
                let width = self.bounds().size.width;
                let mut y = 0.0_f64;
                for row in self.ivars().rows.borrow().iter() {
                    let row_h = row.intrinsic_row_size().height;
                    row.setFrame(CGRect::new(CGPoint::new(0.0, y), CGSize::new(width, row_h)));
                    y += row_h + RADIO_ROW_SPACING;
                }
            });
        }

        // SAFETY: returns the stacked rows' intrinsic size.
        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size(&self) -> CGSize {
            let size = self.intrinsic_size();
            CGSize::new(size.width, size.height)
        }
    }
);

impl RadioGroupView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RadioGroupIvars {
            rows: RefCell::new(Vec::new()),
            selected: Cell::new(None),
            enabled: Cell::new(true),
            colors: Rc::new(RefCell::new(RadioColors::default())),
        });
        // SAFETY: `initWithFrame:` is `UIView`'s designated initializer.
        unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] }
    }

    fn set_items(
        &self,
        titles: &[String],
        font: Option<&UIFont>,
        shared: &Rc<RefCell<Shared>>,
        targets: &RefCell<Vec<ActionTarget>>,
    ) {
        let mtm = self.mtm();
        for row in self.ivars().rows.borrow_mut().drain(..) {
            view::remove_from_superview(&row);
        }
        targets.borrow_mut().clear();
        let enabled = self.ivars().enabled.get();
        let selected = self.ivars().selected.get();
        let mut rows = Vec::with_capacity(titles.len());
        for (index, title) in titles.iter().enumerate() {
            let row = RadioRowView::new(mtm, title, font, Rc::clone(&self.ivars().colors));
            row.set_enabled_row(enabled);
            row.set_selected_row(Some(index) == selected);
            let shared = Rc::clone(shared);
            let control: &UIControl = &row;
            targets.borrow_mut().push(ActionTarget::new(
                control,
                ControlEvents::TOUCH_UP_INSIDE,
                move |_mtm| {
                    if let Some(handler) = &shared.borrow().handler {
                        handler(index);
                    }
                },
            ));
            self.addSubview(&row);
            rows.push(row);
        }
        *self.ivars().rows.borrow_mut() = rows;
        self.invalidateIntrinsicContentSize();
        self.setNeedsLayout();
    }

    fn set_selected_index(&self, index: Option<usize>) {
        self.ivars().selected.set(index);
        for (i, row) in self.ivars().rows.borrow().iter().enumerate() {
            row.set_selected_row(Some(i) == index);
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.ivars().selected.get()
    }

    fn set_font(&self, font: &UIFont) {
        for row in self.ivars().rows.borrow().iter() {
            row.set_font(font);
        }
        self.invalidateIntrinsicContentSize();
        self.setNeedsLayout();
    }

    fn set_enabled(&self, enabled: bool) {
        self.ivars().enabled.set(enabled);
        for row in self.ivars().rows.borrow().iter() {
            row.set_enabled_row(enabled);
        }
    }

    fn set_colors(&self, accent: &UIColor, foreground: &UIColor, muted: &UIColor) {
        *self.ivars().colors.borrow_mut() = RadioColors {
            accent: accent.into(),
            foreground: foreground.into(),
            muted: muted.into(),
        };
        for row in self.ivars().rows.borrow().iter() {
            row.apply_visual_state();
        }
    }

    fn intrinsic_size(&self) -> Size {
        let rows = self.ivars().rows.borrow();
        let mut max_w = 0.0_f64;
        let mut total_h = 0.0_f64;
        for (index, row) in rows.iter().enumerate() {
            let size = row.intrinsic_row_size();
            max_w = max_w.max(size.width);
            total_h += size.height;
            if index + 1 < rows.len() {
                total_h += RADIO_ROW_SPACING;
            }
        }
        Size::new(max_w, total_h)
    }
}

/// A selection menu over one of `UIKit`'s selection controls.
///
/// The wrapped control is chosen by [`PickerStyle`] at construction and
/// stays fixed; [`Picker::view`] is what a container adds and frames.
pub struct Picker {
    control: Control,
    mtm: MainThreadMarker,
    /// Action targets for segmented and radio styles; the menu's `UIAction`s
    /// and the wheel's delegate call the handler directly.
    targets: RefCell<Vec<ActionTarget>>,
    /// The menu style's current `UIAction` children, kept for check-state
    /// updates.
    actions: RefCell<Vec<Retained<UIAction>>>,
    /// The wheel style's delegate; `Some` only for [`PickerStyle::Wheel`].
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
    Radio(Retained<RadioGroupView>),
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
            Self::Radio(radio) => radio,
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
                let button = crate::uikit::button::CocoaUiButton::new(mtm).into_super();
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
                let group = RadioGroupView::new(mtm);
                (Control::Radio(group), None)
            }
            PickerStyle::Wheel => {
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
            Control::Radio(group) => {
                let font = self.shared.borrow().font.clone();
                group.set_items(titles, font.as_deref(), &self.shared, &self.targets);
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
            Control::Radio(group) => group.set_selected_index(index),
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
            Control::Radio(group) => group.selected_index(),
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
    /// labels, radio rows, and menu button's title adopt it.
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
            Control::Radio(group) => group.set_font(font),
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

    /// Applies semantic theme colours to the radio group; no-op for other styles.
    pub fn set_radio_colors(&self, accent: &UIColor, foreground: &UIColor, muted: &UIColor) {
        if let Control::Radio(group) = &self.control {
            group.set_colors(accent, foreground, muted);
        }
    }

    /// How large the control is drawn.
    ///
    /// `UIKit` has no size classes for its selection controls: the value is
    /// ignored.
    pub const fn set_control_size(&self, _size: ControlSize) {}

    /// Whether the control responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        match &self.control {
            Control::Menu(button) => button.setEnabled(enabled),
            Control::Segmented(segmented) => segmented.setEnabled(enabled),
            Control::Radio(group) => group.set_enabled(enabled),
            Control::Wheel(wheel) => wheel.setUserInteractionEnabled(enabled),
        }
    }

    /// The size a measure pass reports — the control's intrinsic size.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        match &self.control {
            Control::Radio(group) => group.intrinsic_size(),
            control => {
                let size = control.view().intrinsicContentSize();
                Size::new(size.width, size.height)
            }
        }
    }
}
