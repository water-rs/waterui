//! The `date_picker` leaf: `Native<DatePickerConfig>` rendered as a row of
//! the visible label, the platform date-time control, and on `UIKit` the
//! seconds readout plus stepper `WuiDatePicker` adds for second-precision
//! types.
//!
//! Mirrors `WuiDatePicker`: the `value` `Binding<DateTime>` is two-way —
//! watchers push the date onto the control and the control's action merges
//! the edited fields back, keeping the components the picker hides
//! (`merged_value`). A `syncing` flag drops the echo each direction raises.
//! `jiff`'s full-range sentinels (`-9999-01-01` / `9999-12-31`) become nil
//! `minimumDate`/`maximumDate` bounds, matching `nativeRangeBound`.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use cocoa_ui::date::{DateParts, DateTimeParts, TimeParts};
use cocoa_ui::{PlatformView, Rect, Retained};
use waterui::component::form::picker::date::{DatePickerConfig, DatePickerType, DateTime};
use waterui::reactive::{Binding, Signal};
use waterui::resolve::Resolvable;
use waterui_core::interaction::Disabled;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf, RenderContext};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use super::DatePickerType;
    pub(super) use cocoa_ui::appkit::{DatePicker, DatePickerElements, HostView};

    /// The element flags a `DatePickerType` selects on `NSDatePicker`.
    pub(super) const fn elements(ty: DatePickerType) -> DatePickerElements {
        match ty {
            DatePickerType::Date => DatePickerElements::Date,
            DatePickerType::HourAndMinute => DatePickerElements::HourMinute,
            DatePickerType::HourMinuteAndSecond => DatePickerElements::HourMinuteSecond,
            DatePickerType::DateHourAndMinute => DatePickerElements::DateHourMinute,
            DatePickerType::DateHourMinuteAndSecond => DatePickerElements::DateHourMinuteSecond,
        }
    }
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use super::DatePickerType;
    pub(super) use cocoa_ui::objc2_ui_kit::{NSTextAlignment, UIColor};
    pub(super) use cocoa_ui::uikit::label::Label;
    pub(super) use cocoa_ui::uikit::{DatePicker, DatePickerMode, HostView, Stepper};

    /// The mode a `DatePickerType` selects on `UIDatePicker`.
    pub(super) const fn mode(ty: DatePickerType) -> DatePickerMode {
        match ty {
            DatePickerType::Date => DatePickerMode::Date,
            DatePickerType::HourAndMinute | DatePickerType::HourMinuteAndSecond => {
                DatePickerMode::Time
            }
            DatePickerType::DateHourAndMinute | DatePickerType::DateHourMinuteAndSecond => {
                DatePickerMode::DateAndTime
            }
        }
    }
}

use platform::{DatePicker, HostView};

/// The control as its platform view.
fn as_view(picker: &DatePicker) -> &PlatformView {
    picker
}

/// The gap between the row's children.
const SPACING: f64 = 8.0;

/// Whether the type edits seconds — `UIKit` shows the seconds label and
/// stepper for these; `AppKit` folds them into the control's elements.
#[cfg(any(target_os = "ios", test))]
const fn shows_seconds(ty: DatePickerType) -> bool {
    matches!(
        ty,
        DatePickerType::HourMinuteAndSecond | DatePickerType::DateHourMinuteAndSecond
    )
}

/// The `jiff` value as the kit's plain parts.
fn parts_of(value: DateTime) -> DateTimeParts {
    DateTimeParts {
        date: DateParts {
            year: i32::from(value.year()),
            month: i32::from(value.month()),
            day: i32::from(value.day()),
        },
        time: TimeParts {
            hour: i32::from(value.hour()),
            minute: i32::from(value.minute()),
            second: i32::from(value.second()),
        },
    }
}

/// `DateTimeParts` back into `jiff` — a binding write.
///
/// # Panics
///
/// When the parts fall outside `jiff`'s range — impossible for control
/// output, matching the Swift `fatalError`.
fn to_jiff(parts: DateTimeParts) -> DateTime {
    let date = waterui::component::form::picker::date::Date::new(
        i16::try_from(parts.date.year).expect("year in range"),
        i8::try_from(parts.date.month).expect("month in range"),
        i8::try_from(parts.date.day).expect("day in range"),
    )
    .expect("picker date parts are always valid");
    date.at(
        i8::try_from(parts.time.hour).expect("hour in range"),
        i8::try_from(parts.time.minute).expect("minute in range"),
        i8::try_from(parts.time.second).expect("second in range"),
        0,
    )
}

/// The `NSDate` `value` names — `wuiDateTimeToDate`, `fatalError` included.
///
/// # Panics
///
/// When the Gregorian calendar cannot resolve `value`.
fn ns_date(value: DateTime) -> Retained<objc2_foundation::NSDate> {
    cocoa_ui::date::ns_date(&parts_of(value))
        .expect("WaterUI DatePicker cannot represent the date-time")
}

/// Whether `value` is a `jiff` full-range sentinel — `nil` on the platform
/// control (`nativeRangeBound`).
fn is_full_range_bound(value: DateTime) -> bool {
    let parts = parts_of(value);
    parts.date
        == (DateParts {
            year: -9999,
            month: 1,
            day: 1,
        })
        && parts.time == TimeParts::default()
        || parts.date
            == (DateParts {
                year: 9999,
                month: 12,
                day: 31,
            })
            && parts.time
                == (TimeParts {
                    hour: 23,
                    minute: 59,
                    second: 59,
                })
}

/// A range bound as `NSDate`; `None` for the `jiff` full-range sentinels.
fn range_bound(value: DateTime) -> Option<Retained<objc2_foundation::NSDate>> {
    (!is_full_range_bound(value)).then(|| ns_date(value))
}

/// The parts the control's `NSDate` carries — `NativeDateComponents`'s
/// fatal-on-incomplete check.
///
/// # Panics
///
/// When the Gregorian calendar cannot resolve `date`.
fn read_parts(date: &objc2_foundation::NSDate) -> DateTimeParts {
    cocoa_ui::date::date_time_parts(date)
        .expect("Gregorian calendar failed to resolve DatePicker components")
}

/// The binding value after the user edited the control: the fields `ty`
/// shows take `picker`'s components — seconds take `seconds_override` (the
/// `UIKit` stepper); the fields `ty` hides keep `current`'s values
/// (`mergedValue`).
fn merged_value(
    ty: DatePickerType,
    picker: DateTimeParts,
    seconds_override: Option<i32>,
    current: DateTime,
) -> DateTimeParts {
    let mut merged = parts_of(current);
    match ty {
        DatePickerType::Date => {
            merged.date = picker.date;
        }
        DatePickerType::HourAndMinute => {
            merged.time.hour = picker.time.hour;
            merged.time.minute = picker.time.minute;
        }
        DatePickerType::HourMinuteAndSecond => {
            merged.time.hour = picker.time.hour;
            merged.time.minute = picker.time.minute;
            merged.time.second = seconds_override.unwrap_or(picker.time.second);
        }
        DatePickerType::DateHourAndMinute => {
            merged.date = picker.date;
            merged.time.hour = picker.time.hour;
            merged.time.minute = picker.time.minute;
        }
        DatePickerType::DateHourMinuteAndSecond => {
            merged.date = picker.date;
            merged.time.hour = picker.time.hour;
            merged.time.minute = picker.time.minute;
            merged.time.second = seconds_override.unwrap_or(picker.time.second);
        }
    }
    merged
}

/// The `UIKit` seconds column: `":%02d"` readout plus the 0–59 stepper,
/// with the themed font/color kept so the text can be rebuilt in place.
#[cfg(target_os = "ios")]
struct SecondsControls {
    /// The right-aligned `":SS"` label.
    label: Retained<platform::Label>,
    /// The 0–59 stepper.
    stepper: Retained<platform::Stepper>,
    /// The body's current point size.
    font_size: Cell<f64>,
    /// The body's current platform weight.
    font_weight: Cell<f64>,
    /// The `Foreground` color the readout draws in.
    foreground: RefCell<Retained<platform::UIColor>>,
    /// The second value last shown — text rebuilds consult it.
    value: Cell<i32>,
}

/// The leaf's live state.
struct DatePickerState {
    /// The platform control.
    picker: Retained<DatePicker>,
    /// `true` while the binding is being pushed onto the control — the
    /// action the push echoes must not write back (`isSyncingFromBinding`).
    syncing: Cell<bool>,
    /// The picker type, for the merge.
    ty: DatePickerType,
    /// The rendered label child.
    label: Mounted,
    /// The seconds column — `UIKit`, second-precision types only.
    #[cfg(target_os = "ios")]
    seconds: Option<SecondsControls>,
}

/// Rebuilds the `":SS"` readout with the current themed font and color —
/// `applySecondsFont` plus the text.
#[cfg(target_os = "ios")]
fn set_seconds_text(controls: &SecondsControls) {
    let text = alloc::format!(":{:02}", controls.value.get());
    let font = cocoa_ui::font::monospaced_digit(
        cocoa_ui::MainThreadMarker::from(&*controls.label),
        controls.font_size.get(),
        controls.font_weight.get(),
    );
    let foreground = controls.foreground.borrow();
    let attributed = cocoa_ui::text::build(
        cocoa_ui::MainThreadMarker::from(&*controls.label),
        &[cocoa_ui::text::TextRun {
            text: &text,
            font: &font,
            foreground: Some(&foreground),
            background: None,
            underline: false,
            strikethrough: false,
            letter_spacing: 0.0,
            line_height: 0.0,
        }],
    );
    controls.label.set_attributed_text(&attributed);
}

/// Mirrors `syncControls(with:)` — push `value` onto the control and, on
/// `UIKit`, the seconds controls.
fn sync_controls(state: &DatePickerState, value: DateTime) {
    state.picker.set_date(&ns_date(value));
    #[cfg(target_os = "ios")]
    if let Some(seconds) = &state.seconds {
        seconds.value.set(i32::from(value.second()));
        seconds.stepper.set_value(f64::from(value.second()));
        set_seconds_text(seconds);
    }
}

/// The stepper's reported seconds — clamped to its range before the
/// `Int(...)`-style truncation, so the cast cannot lose value.
#[cfg(target_os = "ios")]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the value is clamped to 0...59 first"
)]
const fn current_seconds(value: f64) -> i32 {
    value.clamp(0.0, 59.0) as i32
}

/// Writes the control's date — seconds from the stepper on `UIKit` —
/// merged onto the binding's hidden fields (`updateBindingFromControls`).
fn update_binding(state: &Rc<RefCell<DatePickerState>>, binding: &Binding<DateTime>) {
    let borrowed = state.borrow();
    if borrowed.syncing.get() {
        return;
    }
    let picker_parts = read_parts(&borrowed.picker.date());
    #[cfg(target_os = "ios")]
    let seconds_override = borrowed
        .seconds
        .as_ref()
        .map(|seconds| current_seconds(seconds.stepper.value()));
    #[cfg(not(target_os = "ios"))]
    let seconds_override = None;
    let merged = merged_value(
        borrowed.ty,
        picker_parts,
        seconds_override,
        binding.snapshot(),
    );
    drop(borrowed);
    binding.set(to_jiff(merged));
}

/// Lays out the row inside `view`'s bounds: label leading and centered,
/// the seconds column hugging the trailing edge on `UIKit`, and the picker
/// filling the space between — the constraint layout's middle spring.
fn layout_children(view: &PlatformView, state: &DatePickerState) {
    let bounds = cocoa_ui::view::bounds(view);
    let width = bounds.size.width;
    let height = bounds.size.height;
    let rtl = cocoa_ui::view::is_right_to_left(view);

    let place = |child: &PlatformView, x: f64, y: f64, w: f64, h: f64| {
        let x = if rtl { width - x - w } else { x };
        cocoa_ui::view::set_frame(child, Rect::new(x, y, w, h));
    };

    let label_size = state.label.layout().measure(ProposalSize::UNSPECIFIED).size;
    let label_w = f64::from(label_size.width);
    let has_label = label_size.width > 0.0 && label_size.height > 0.0;
    let picker_x = label_w + if has_label { SPACING } else { 0.0 };

    #[cfg(target_os = "ios")]
    let seconds_width = state.seconds.as_ref().map_or(0.0, |seconds| {
        let label_metrics = seconds.label.measure(cocoa_ui::text::WrapWidth::Free);
        let stepper_size = seconds.stepper.intrinsic_size();
        let stepper_x = width - stepper_size.width;
        place(
            &seconds.stepper,
            stepper_x,
            height / 2.0 - stepper_size.height / 2.0,
            stepper_size.width,
            stepper_size.height,
        );
        let label_size = label_metrics.size;
        place(
            &seconds.label,
            stepper_x - SPACING - label_size.width,
            height / 2.0 - label_size.height / 2.0,
            label_size.width,
            label_size.height,
        );
        SPACING + label_size.width + SPACING + stepper_size.width
    });
    #[cfg(not(target_os = "ios"))]
    let seconds_width = 0.0;

    let picker_size = state.picker.intrinsic_size();
    place(
        as_view(&state.picker),
        picker_x,
        height / 2.0 - picker_size.height / 2.0,
        (width - picker_x - seconds_width).max(0.0),
        picker_size.height,
    );

    if has_label {
        let label_h = f64::from(label_size.height);
        place(
            state.label.view(),
            0.0,
            height / 2.0 - label_h / 2.0,
            label_w,
            label_h,
        );
    }
}

/// The container's layout face: picker intrinsic plus the seconds column,
/// plus spacing and the label when it measures non-empty — `sizeThatFits`.
struct DatePickerSubView {
    state: Rc<RefCell<DatePickerState>>,
}

impl core::fmt::Debug for DatePickerSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DatePickerSubView").finish_non_exhaustive()
    }
}

impl SubView for DatePickerSubView {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        let state = self.state.borrow();
        let label_size = state.label.layout().measure(ProposalSize::UNSPECIFIED).size;
        let picker_size = state.picker.intrinsic_size();
        #[cfg(target_os = "ios")]
        let seconds_width = state.seconds.as_ref().map_or(0.0, |seconds| {
            SPACING
                + seconds
                    .label
                    .measure(cocoa_ui::text::WrapWidth::Free)
                    .size
                    .width
                + SPACING
                + seconds.stepper.intrinsic_size().width
        });
        #[cfg(not(target_os = "ios"))]
        let seconds_width = 0.0;

        let has_label = label_size.width > 0.0 && label_size.height > 0.0;
        let mut width = picker_size.width + seconds_width;
        let mut height = picker_size.height;
        if has_label {
            width += SPACING + f64::from(label_size.width);
            height = height.max(f64::from(label_size.height));
        }
        ViewDimensions::new(Size::new(width as f32, height as f32))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Installs the `date_picker` handler on the dispatcher:
/// `Native<DatePickerConfig>` maps to the label/control row.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<DatePickerConfig>(|config, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let host_view: &PlatformView = &host;

        #[cfg(target_os = "macos")]
        let picker = DatePicker::new(mtm, platform::elements(config.ty));
        #[cfg(target_os = "ios")]
        let picker = DatePicker::new(mtm, platform::mode(config.ty));
        picker.set_range(
            range_bound(*config.range.start()).as_deref(),
            range_bound(*config.range.end()).as_deref(),
        );
        cocoa_ui::view::add_subview(host_view, as_view(&picker));

        let label = ctx
            .render(waterui_backend_core::AnyView::new(config.label.clone()))
            .mount(host_view);
        cocoa_ui::view::hide_from_accessibility(label.view());

        let state = Rc::new(RefCell::new(DatePickerState {
            picker,
            syncing: Cell::new(false),
            ty: config.ty,
            label,
            #[cfg(target_os = "ios")]
            seconds: None,
        }));

        // `syncControls` runs before the action is wired, like the Swift init.
        sync_controls(&state.borrow(), config.value.snapshot());

        #[cfg(target_os = "ios")]
        if shows_seconds(config.ty) {
            let seconds_label = platform::Label::new(mtm);
            seconds_label.set_text_alignment(platform::NSTextAlignment::Right);
            let stepper = platform::Stepper::new(mtm);
            stepper.set_range(0.0, 59.0);
            stepper.set_step(1.0);
            cocoa_ui::view::add_subview(host_view, &seconds_label);
            cocoa_ui::view::add_subview(host_view, &stepper);
            state.borrow_mut().seconds = Some(SecondsControls {
                label: seconds_label,
                stepper,
                font_size: Cell::new(0.0),
                font_weight: Cell::new(cocoa_ui::font::weight::REGULAR),
                foreground: RefCell::new(platform::UIColor::labelColor()),
                value: Cell::new(0),
            });
            sync_controls(&state.borrow(), config.value.snapshot());
        }

        // Control → binding: the action merges the edited fields back.
        let action_target = state.borrow().picker.install_action({
            let state = Rc::clone(&state);
            let binding = config.value.clone();
            move || update_binding(&state, &binding)
        });

        // `secondsChanged`: refresh the readout, then merge.
        #[cfg(target_os = "ios")]
        let seconds_target = state.borrow().seconds.as_ref().map(|seconds| {
            seconds.stepper.install_action({
                let state = Rc::clone(&state);
                let binding = config.value.clone();
                move |value| {
                    {
                        let borrowed = state.borrow();
                        if borrowed.syncing.get() {
                            return;
                        }
                        if let Some(controls) = &borrowed.seconds {
                            controls.value.set(current_seconds(value));
                            set_seconds_text(controls);
                        }
                    }
                    update_binding(&state, &binding);
                }
            })
        });

        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |view| layout_children(view, &state.borrow())
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            DatePickerSubView {
                state: Rc::clone(&state),
            },
        );
        wire(&mut leaf, &config, &state, ctx, mtm);

        // Binding → control; the syncing flag drops the echo our own write
        // raises back through the action.

        leaf.keep(action_target);
        #[cfg(target_os = "ios")]
        leaf.keep(seconds_target);
        leaf.keep(state);
        leaf
    });
}

/// Wires the leaf's watches and binds — control ← signal subscriptions
/// that mirror `WuiDatePicker`'s observations.
fn wire(
    leaf: &mut NativeLeaf,
    config: &DatePickerConfig,
    state: &Rc<RefCell<DatePickerState>>,
    ctx: &RenderContext<'_>,
    mtm: cocoa_ui::MainThreadMarker,
) {
    // Binding → control; the syncing flag drops the echo our own write
    // raises back through the action.
    leaf.watch(&config.value, {
        let state = Rc::clone(state);
        move |ctx| {
            let borrowed = state.borrow();
            if borrowed.syncing.get() {
                return;
            }
            borrowed.syncing.set(true);
            sync_controls(&borrowed, *ctx.value());
            borrowed.syncing.set(false);
        }
    });

    // The body font themes the control's text on `AppKit`, and the
    // seconds readout's monospaced-digit face on `UIKit`.
    let body = waterui::text::font::Body.resolve(ctx.env());
    leaf.bind(&body, {
        let state = Rc::clone(state);
        move |font: waterui::text::font::ResolvedFont| {
            #[cfg(target_os = "macos")]
            state.borrow().picker.set_font(&platform_font(mtm, &font));
            #[cfg(target_os = "ios")]
            {
                let _ = mtm;
                let borrowed = state.borrow();
                if let Some(seconds) = &borrowed.seconds {
                    seconds.font_size.set(f64::from(font.size));
                    seconds.font_weight.set(platform_weight(font.weight));
                    set_seconds_text(seconds);
                }
            }
        }
    });

    // `Foreground` themes the seconds readout on `UIKit`.
    #[cfg(target_os = "ios")]
    {
        let foreground = waterui::theme::color::Foreground.resolve(ctx.env());
        leaf.bind(&foreground, {
            let state = Rc::clone(state);
            move |color: waterui::graphics::color::WorkingColor| {
                let borrowed = state.borrow();
                if let Some(seconds) = &borrowed.seconds {
                    *seconds.foreground.borrow_mut() = platform_color(&color);
                    set_seconds_text(seconds);
                }
            }
        });
    }

    // The label's semantic text is spoken on the control (and the
    // stepper on `UIKit`), the `WuiControlAccessibility` treatment.
    let accessibility_label = config.label.accessibility_label();
    leaf.bind(&accessibility_label, {
        let state = Rc::clone(state);
        move |styled| {
            let text = cocoa_ui::text::strip_bidi_controls(&styled.to_plain());
            let borrowed = state.borrow();
            borrowed.picker.set_accessibility_label(Some(&text));
            #[cfg(target_os = "ios")]
            if let Some(seconds) = &borrowed.seconds {
                seconds.stepper.set_accessibility_label(Some(&text));
            }
        }
    });

    // A disabled subtree must not respond to input.
    if let Some(disabled) = ctx.env().get::<Disabled>() {
        leaf.bind(disabled.signal(), {
            let state = Rc::clone(state);
            move |is_disabled: bool| {
                let borrowed = state.borrow();
                borrowed.picker.set_enabled(!is_disabled);
                #[cfg(target_os = "ios")]
                if let Some(seconds) = &borrowed.seconds {
                    seconds.stepper.set_enabled(!is_disabled);
                }
            }
        });
    }
}

/// The platform weight of a `FontWeight` on the `NSFont`/`UIFont` scale.
const fn platform_weight(weight: waterui::text::font::FontWeight) -> f64 {
    use cocoa_ui::font::weight;
    use waterui::text::font::FontWeight;
    match weight {
        FontWeight::Thin => weight::THIN,
        FontWeight::UltraLight => weight::ULTRA_LIGHT,
        FontWeight::Light => weight::LIGHT,
        FontWeight::Normal => weight::REGULAR,
        FontWeight::Medium => weight::MEDIUM,
        FontWeight::SemiBold => weight::SEMI_BOLD,
        FontWeight::Bold => weight::BOLD,
        FontWeight::UltraBold => weight::HEAVY,
        FontWeight::Black => weight::BLACK,
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object — the
/// `toUIColor`/`toNSColor` semantics sibling ports use.
#[cfg(target_os = "ios")]
fn platform_color(color: &waterui::graphics::color::WorkingColor) -> Retained<platform::UIColor> {
    {
        let [red, green, blue, alpha] = color.components;
        cocoa_ui::uikit::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// The platform face a resolved font names — the `Body` resolved font
/// applied to the control's text.
///
/// # Panics
///
/// When the family names no installed font and no generic.
#[cfg(target_os = "macos")]
fn platform_font(
    mtm: cocoa_ui::MainThreadMarker,
    resolved: &waterui::text::font::ResolvedFont,
) -> Retained<cocoa_ui::Font> {
    use waterui::text::font::FontDesign;
    let size = f64::from(resolved.size);
    let weight = platform_weight(resolved.weight);
    match resolved.family.as_deref() {
        Some(family) if !family.is_empty() => {
            let mut resolved_font = None;
            for candidate in family
                .split(',')
                .map(str::trim)
                .filter(|candidate| !candidate.is_empty())
            {
                resolved_font = match candidate {
                    "system" | "sans-serif" => Some(cocoa_ui::font::system(mtm, size, weight)),
                    _ => cocoa_ui::font::named(candidate, size),
                };
                if resolved_font.is_some() {
                    break;
                }
            }
            resolved_font.unwrap_or_else(|| panic!("WaterUI: font family '{family}' not found"))
        }
        _ => match resolved.design {
            FontDesign::Default => cocoa_ui::font::system(mtm, size, weight),
            FontDesign::Monospaced => cocoa_ui::font::monospaced(mtm, size, weight),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui::component::form::picker::date::Date;

    fn dt(year: i32, month: i32, day: i32, hour: i32, minute: i32, second: i32) -> DateTimeParts {
        DateTimeParts {
            date: DateParts { year, month, day },
            time: TimeParts {
                hour,
                minute,
                second,
            },
        }
    }

    #[test]
    fn merged_value_keeps_hidden_fields() {
        let current = Date::new(2020, 2, 3).unwrap().at(4, 5, 6, 0);
        let picker = dt(2030, 11, 12, 13, 14, 15);

        let merged = merged_value(DatePickerType::Date, picker, None, current);
        assert_eq!(merged.date, picker.date);
        assert_eq!(merged.time, parts_of(current).time);

        let merged = merged_value(DatePickerType::HourAndMinute, picker, None, current);
        assert_eq!(merged.date, parts_of(current).date);
        assert_eq!(merged.time.hour, 13);
        assert_eq!(merged.time.minute, 14);
        assert_eq!(merged.time.second, 6);

        let merged = merged_value(
            DatePickerType::HourMinuteAndSecond,
            picker,
            Some(42),
            current,
        );
        assert_eq!(merged.time.second, 42);

        let merged = merged_value(
            DatePickerType::DateHourMinuteAndSecond,
            picker,
            Some(59),
            current,
        );
        assert_eq!(merged.date, picker.date);
        assert_eq!(
            merged.time,
            TimeParts {
                hour: 13,
                minute: 14,
                second: 59
            }
        );
    }

    #[test]
    fn full_range_sentinels_clear_bounds() {
        let min = Date::new(-9999, 1, 1).unwrap().at(0, 0, 0, 0);
        let max = Date::new(9999, 12, 31).unwrap().at(23, 59, 59, 0);
        assert!(is_full_range_bound(min));
        assert!(is_full_range_bound(max));
        assert!(range_bound(min).is_none());
        assert!(range_bound(max).is_none());

        let ordinary = Date::new(2025, 6, 15).unwrap().at(12, 0, 0, 0);
        assert!(!is_full_range_bound(ordinary));
        assert!(range_bound(ordinary).is_some());
    }

    #[test]
    fn jiff_parts_round_trip() {
        let value = Date::new(2025, 3, 9).unwrap().at(22, 8, 30, 0);
        assert_eq!(to_jiff(parts_of(value)), value);
    }

    #[test]
    fn seconds_types_match_swift_switch() {
        assert!(!shows_seconds(DatePickerType::Date));
        assert!(!shows_seconds(DatePickerType::HourAndMinute));
        assert!(shows_seconds(DatePickerType::HourMinuteAndSecond));
        assert!(!shows_seconds(DatePickerType::DateHourAndMinute));
        assert!(shows_seconds(DatePickerType::DateHourMinuteAndSecond));
    }
}
