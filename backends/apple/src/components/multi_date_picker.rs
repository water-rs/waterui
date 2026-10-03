//! The `multi_date_picker` leaf: the `MultiDatePicker` composer claimed
//! before `body()` expands it, rendered as the label on top of the
//! platform multi-date control.
//!
//! `MultiDatePickerConfig` never reaches `Native` under this waterui pin —
//! the composer's `body()` produces `MultiDatePickerFallback`, a plain
//! `View` — so the port registers `MultiDatePicker` itself and replicates
//! `body()` exactly: a `Hook<MultiDatePickerConfig>` in the environment
//! wins and its result is rendered; otherwise the label resolves and the
//! config renders here. `Native<MultiDatePickerConfig>` is claimed too,
//! for the payload a hook emits.
//!
//! Mirrors `WuiMultiDatePicker`: `value` is a `Binding<Vec<Date>>` toggled
//! per tap (`applySelectionToggle` removes a selected day or appends it
//! sorted by `dateKey`), `decorated` is a passive `Computed<Vec<Date>>`
//! drawn as the small `MutedForeground` dot, and both signals push into
//! the control on every change (`syncFromModel`). On `UIKit` the control
//! is a `UICalendarView` under the label in a vertical stack; on `AppKit`
//! it is an `NSDatePicker` plus an add/remove symbol button and a textual
//! list of the selected dates.

use alloc::collections::BTreeSet;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::ops::RangeInclusive;

use cocoa_ui::date::DateParts;
use cocoa_ui::{PlatformView, Rect, Retained};
use waterui::component::form::picker::date::Date;
use waterui::component::form::picker::multi_date::{MultiDatePicker, MultiDatePickerConfig};
use waterui::reactive::{Binding, Computed, Signal};
use waterui::resolve::Resolvable;
use waterui::view::{ConfigurableView, Hook};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf, RenderContext};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::label::Label;
    pub(super) use cocoa_ui::appkit::{Button, DatePicker, DatePickerElements, HostView};
    pub(super) use cocoa_ui::objc2_app_kit::NSColor;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::objc2_ui_kit::UIColor;
    pub(super) use cocoa_ui::uikit::{CalendarView, HostView};
}

use platform::HostView;

/// The vertical gap between the stack's children.
const SPACING: f64 = 8.0;
/// The `AppKit` selection list's own spacing.
#[cfg(target_os = "macos")]
const ROW_SPACING: f64 = 4.0;

/// The `jiff` `Date` as the kit's plain parts — `dateKey`'s value.
fn parts_of(value: Date) -> DateParts {
    DateParts {
        year: i32::from(value.year()),
        month: i32::from(value.month()),
        day: i32::from(value.day()),
    }
}

/// `DateParts` back into `jiff` — `requireDate`/`currentDate`, whose
/// `fatalError` on incomplete components this matches.
///
/// # Panics
///
/// When the parts are not a valid calendar date.
fn to_jiff(parts: DateParts) -> Date {
    Date::new(
        i16::try_from(parts.year).expect("year in range"),
        i8::try_from(parts.month).expect("month in range"),
        i8::try_from(parts.day).expect("day in range"),
    )
    .expect("WaterUI received an invalid calendar date")
}

/// The `NSDate` `value` names — `toDate`, `fatalError` included.
///
/// # Panics
///
/// When the Gregorian calendar cannot resolve `value`.
fn ns_date(value: Date) -> Retained<objc2_foundation::NSDate> {
    cocoa_ui::date::ns_date_from_date_parts(parts_of(value))
        .expect("WaterUI received an invalid calendar date")
}

/// `applySelectionToggle`'s core: `current` leaves the list when present,
/// else joins it sorted by `dateKey`.
fn toggle_selection(selected: &mut Vec<Date>, current: Date) {
    let key = parts_of(current);
    if let Some(index) = selected.iter().position(|day| parts_of(*day) == key) {
        selected.remove(index);
    } else {
        selected.push(current);
        selected.sort_by_key(|day| parts_of(*day));
    }
}

/// `applySelectionToggle` applied to the binding.
fn apply_selection_toggle(binding: &Binding<Vec<Date>>, current: Date) {
    let mut selected = binding.snapshot();
    toggle_selection(&mut selected, current);
    binding.set(selected);
}

/// `canToggle`: the day is inside the inclusive range.
#[cfg(any(target_os = "ios", test))]
fn can_toggle(range: &RangeInclusive<Date>, parts: DateParts) -> bool {
    let Ok(day) = Date::new(
        i16::try_from(parts.year).unwrap_or(0),
        i8::try_from(parts.month).unwrap_or(1),
        i8::try_from(parts.day).unwrap_or(1),
    ) else {
        return false;
    };
    range.start() <= &day && &day <= range.end()
}

/// The leaf's live state, shared between the watchers, the layout handler
/// and the platform control's callbacks.
struct MultiState {
    /// The host container — row labels mount onto it on `AppKit`.
    #[cfg(target_os = "macos")]
    host: Retained<PlatformView>,
    /// Proof of the main thread, for building labels in `syncFromModel`.
    #[cfg(target_os = "macos")]
    mtm: cocoa_ui::MainThreadMarker,
    /// The rendered label child — the stack's first row.
    label: Mounted,
    /// The two-way selection the control writes into.
    value: Binding<Vec<Date>>,
    /// The passive decorations.
    decorated: Computed<Vec<Date>>,
    /// The valid range, for `canToggle`.
    range: RangeInclusive<Date>,
    /// `decorated` as a `dateKey` set — `decoratedDateKeys`.
    decorated_keys: Rc<RefCell<BTreeSet<DateParts>>>,
    /// The `UICalendarView` on `UIKit`.
    #[cfg(target_os = "ios")]
    calendar: platform::CalendarView,
    /// The decoration's current `MutedForeground` color.
    #[cfg(target_os = "ios")]
    decoration_color: Rc<RefCell<Retained<platform::UIColor>>>,
    /// The single-date `NSDatePicker` on `AppKit`.
    #[cfg(target_os = "macos")]
    picker: Retained<platform::DatePicker>,
    /// The add/remove symbol button on `AppKit`.
    #[cfg(target_os = "macos")]
    toggle: Retained<platform::Button>,
    /// The selection rows on `AppKit`, parallel to the sorted selection.
    #[cfg(target_os = "macos")]
    rows: RefCell<Vec<Retained<platform::Label>>>,
    /// The `Body` face the picker text and rows draw in.
    #[cfg(target_os = "macos")]
    font: RefCell<Retained<cocoa_ui::Font>>,
    /// The `Foreground` color the rows draw in.
    #[cfg(target_os = "macos")]
    foreground: RefCell<Retained<platform::NSColor>>,
}

/// The measured size of a child at its natural width.
fn measure_label(state: &MultiState) -> Size {
    state.label.layout().measure(ProposalSize::UNSPECIFIED).size
}

/// Mirrors `syncFromModel` — the decorated key set plus every
/// platform-specific push.
fn sync_from_model(state: &MultiState) {
    let selected = state.value.snapshot();
    let decorated = state.decorated.snapshot();
    *state.decorated_keys.borrow_mut() = decorated.iter().map(|day| parts_of(*day)).collect();

    #[cfg(target_os = "ios")]
    {
        let selected_parts: Vec<DateParts> = selected.iter().map(|day| parts_of(*day)).collect();
        let decorated_parts: Vec<DateParts> = decorated.iter().map(|day| parts_of(*day)).collect();
        state.calendar.sync(&selected_parts, &decorated_parts);
    }

    #[cfg(target_os = "macos")]
    {
        let current = selected
            .first()
            .copied()
            .unwrap_or_else(|| *state.range.start());
        state.picker.set_date(&ns_date(current));
        state.picker.set_font(&state.font.borrow());
        apply_toggle_glyph(state, current, &selected);

        for row in state.rows.borrow_mut().drain(..) {
            cocoa_ui::view::remove_from_superview(&row);
        }
        let mut rows = state.rows.borrow_mut();
        for day in &selected {
            let row = platform::Label::new(state.mtm);
            let text = formatted(*day, state);
            let font = state.font.borrow();
            let foreground = state.foreground.borrow();
            let attributed = cocoa_ui::text::build(
                state.mtm,
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
            row.set_attributed_text(&attributed);
            cocoa_ui::view::add_subview(&state.host, &row);
            rows.push(row);
        }
    }
}

/// The row's text: medium-format date plus the ` •` decoration suffix.
#[cfg(target_os = "macos")]
fn formatted(day: Date, state: &MultiState) -> alloc::string::String {
    let decorated = state.decorated_keys.borrow().contains(&parts_of(day));
    let suffix = if decorated { " •" } else { "" };
    alloc::format!("{}{}", cocoa_ui::date::format_medium(&ns_date(day)), suffix)
}

/// `applyToggleGlyph`: minus-circle when the picker's date is selected,
/// plus-circle otherwise — icon-only, with the spoken description.
///
/// # Panics
///
/// When the SF Symbol is unavailable — the Swift `fatalError`.
#[cfg(target_os = "macos")]
fn apply_toggle_glyph(state: &MultiState, current: Date, selected: &[Date]) {
    let key = parts_of(current);
    let is_selected = selected.iter().any(|day| parts_of(*day) == key);
    let (symbol, description) = if is_selected {
        ("minus.circle", "Remove Date")
    } else {
        ("plus.circle", "Add Date")
    };
    assert!(
        state.toggle.set_symbol(symbol, description),
        "SF Symbol {symbol} is unavailable"
    );
}

/// The picker's current date on `AppKit` — `currentDate`.
#[cfg(target_os = "macos")]
fn current_date(state: &MultiState) -> Date {
    let parts = cocoa_ui::date::date_parts(&state.picker.date())
        .expect("AppKit date picker returned incomplete date components");
    to_jiff(parts)
}

/// Lays out the stack inside `view`'s bounds, top to bottom.
fn layout_children(view: &PlatformView, state: &MultiState) {
    let bounds = cocoa_ui::view::bounds(view);
    let width = bounds.size.width;

    let label_size = measure_label(state);
    cocoa_ui::view::set_frame(
        state.label.view(),
        Rect::new(0.0, 0.0, width, f64::from(label_size.height)),
    );

    #[cfg(target_os = "ios")]
    {
        let height = bounds.size.height;
        let y = f64::from(label_size.height) + SPACING;
        cocoa_ui::view::set_frame(
            state.calendar.view(),
            Rect::new(0.0, y, width, (height - y).max(0.0)),
        );
    }

    #[cfg(target_os = "macos")]
    {
        let mut y = f64::from(label_size.height) + SPACING;
        let picker_size = state.picker.intrinsic_size();
        cocoa_ui::view::set_frame(
            &state.picker,
            Rect::new(0.0, y, width.max(picker_size.width), picker_size.height),
        );
        y += picker_size.height + SPACING;
        let toggle_size = state.toggle.intrinsic_size();
        cocoa_ui::view::set_frame(
            &state.toggle,
            Rect::new(0.0, y, width.max(toggle_size.width), toggle_size.height),
        );
        y += toggle_size.height + SPACING;
        for (index, row) in state.rows.borrow().iter().enumerate() {
            if index > 0 {
                y += ROW_SPACING;
            }
            let row_size = row.measure(cocoa_ui::text::WrapWidth::Free).size;
            cocoa_ui::view::set_frame(row, Rect::new(0.0, y, width, row_size.height));
            y += row_size.height;
        }
    }
}

/// The container's layout face: the vertical stack's `fittingSize` —
/// children at natural width, their heights summed with the spacings.
struct MultiSubView {
    state: Rc<RefCell<MultiState>>,
}

impl core::fmt::Debug for MultiSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MultiSubView").finish_non_exhaustive()
    }
}

impl SubView for MultiSubView {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let _ = proposal;
        let state = self.state.borrow();
        let label_size = measure_label(&state);
        let mut width = f64::from(label_size.width);
        let mut height = f64::from(label_size.height);

        #[cfg(target_os = "ios")]
        {
            let calendar_size = state.calendar.intrinsic_size();
            width = width.max(calendar_size.width);
            // `WuiMultiDatePicker` measured through
            // `systemLayoutSizeFitting(proposal.width)`: the calendar
            // compresses to the offered width, so the leaf never answers
            // wider — otherwise the scroll chain re-measures at the
            // overflow and the whole content column widens past the
            // viewport.
            if let Some(bound) = proposal.width.filter(|w| w.is_finite() && *w > 0.0) {
                width = width.min(f64::from(bound));
            }
            height += SPACING + calendar_size.height;
        }

        #[cfg(target_os = "macos")]
        {
            let picker_size = state.picker.intrinsic_size();
            let toggle_size = state.toggle.intrinsic_size();
            width = width.max(picker_size.width).max(toggle_size.width);
            height += SPACING + picker_size.height + SPACING + toggle_size.height;
            let rows = state.rows.borrow();
            if !rows.is_empty() {
                height += SPACING;
                for (index, row) in rows.iter().enumerate() {
                    if index > 0 {
                        height += ROW_SPACING;
                    }
                    let row_size = row.measure(cocoa_ui::text::WrapWidth::Free).size;
                    width = width.max(row_size.width);
                    height += row_size.height;
                }
            }
        }

        ViewDimensions::new(Size::new(width as f32, height as f32))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Builds and mounts the platform control — the `UICalendarView` on
/// `UIKit`; the picker, toggle and (lazily) selection rows on `AppKit`.
fn platform_controls(
    host_view: &PlatformView,
    state: &Rc<RefCell<MultiState>>,
    decorated_keys: &Rc<RefCell<BTreeSet<DateParts>>>,
) {
    let borrowed = state.borrow();

    #[cfg(target_os = "ios")]
    {
        let start = ns_date(*borrowed.range.start());
        let end = ns_date(*borrowed.range.end());
        borrowed
            .calendar
            .set_available_range(&cocoa_ui::date::interval(&start, &end));
        borrowed.calendar.set_can_select({
            let range = borrowed.range.clone();
            move |parts| can_toggle(&range, parts)
        });
        borrowed.calendar.on_toggle({
            let value = borrowed.value.clone();
            move |parts| apply_selection_toggle(&value, to_jiff(parts))
        });
        borrowed.calendar.set_decoration({
            let keys = Rc::clone(decorated_keys);
            let color = Rc::clone(&borrowed.decoration_color);
            move |parts| {
                keys.borrow()
                    .contains(&parts)
                    .then(|| color.borrow().clone())
            }
        });
        cocoa_ui::view::add_subview(host_view, borrowed.calendar.view());
    }

    #[cfg(target_os = "macos")]
    {
        let _ = decorated_keys;
        borrowed.picker.set_range(
            Some(&ns_date(*borrowed.range.start())),
            Some(&ns_date(*borrowed.range.end())),
        );
        cocoa_ui::view::add_subview(host_view, &borrowed.picker);
        borrowed.toggle.set_press_handler({
            let state = Rc::clone(state);
            move |_, pressed| {
                if pressed {
                    return;
                }
                let current = current_date(&state.borrow());
                apply_selection_toggle(&state.borrow().value, current);
            }
        });
        cocoa_ui::view::add_subview(host_view, &borrowed.toggle);
    }
}

/// The leaf's watches and binds — every signal subscription
/// `WuiMultiDatePicker` held.
fn wire(
    leaf: &mut NativeLeaf,
    config: &MultiDatePickerConfig,
    state: &Rc<RefCell<MultiState>>,
    ctx: &RenderContext<'_>,
) {
    // `value` and `decorated` both drive `syncFromModel`.
    leaf.watch(&config.value, {
        let state = Rc::clone(state);
        move |_| sync_from_model(&state.borrow())
    });
    leaf.watch(&config.decorated, {
        let state = Rc::clone(state);
        move |_| sync_from_model(&state.borrow())
    });

    #[cfg(target_os = "ios")]
    {
        // `MutedForeground` recolors the decoration dots.
        let muted = waterui::theme::color::MutedForeground.resolve(ctx.env());
        leaf.bind(&muted, {
            let state = Rc::clone(state);
            move |color: waterui::graphics::color::WorkingColor| {
                let borrowed = state.borrow();
                *borrowed.decoration_color.borrow_mut() = platform_color(&color);
                let decorated: Vec<DateParts> = borrowed
                    .decorated
                    .snapshot()
                    .iter()
                    .map(|day| parts_of(*day))
                    .collect();
                borrowed.calendar.reload_decorations(&decorated);
            }
        });
    }

    #[cfg(target_os = "macos")]
    {
        // `Body` themes the picker text and every selection row; a change
        // rebuilds the list — the Swift observation's `syncFromModel`.
        let body = waterui::text::font::Body.resolve(ctx.env());
        leaf.bind(&body, {
            let state = Rc::clone(state);
            move |font: waterui::text::font::ResolvedFont| {
                let mtm = state.borrow().mtm;
                *state.borrow().font.borrow_mut() = platform_font(mtm, &font);
                sync_from_model(&state.borrow());
            }
        });
        let foreground = waterui::theme::color::Foreground.resolve(ctx.env());
        leaf.bind(&foreground, {
            let state = Rc::clone(state);
            move |color: waterui::graphics::color::WorkingColor| {
                *state.borrow().foreground.borrow_mut() = platform_color(&color);
                sync_from_model(&state.borrow());
            }
        });
    }

    // The label's semantic text is spoken on the control — on `AppKit` the
    // picker and the toggle button get it (`additionalTargets`).
    let accessibility_label = config.label.accessibility_label();
    leaf.bind(&accessibility_label, {
        let state = Rc::clone(state);
        move |styled| {
            let text = cocoa_ui::text::strip_bidi_controls(&styled.to_plain());
            let borrowed = state.borrow();
            #[cfg(target_os = "ios")]
            borrowed.calendar.set_accessibility_label(Some(&text));
            #[cfg(target_os = "macos")]
            {
                borrowed.picker.set_accessibility_label(Some(&text));
                borrowed.toggle.set_accessibility_label(Some(&text));
            }
        }
    });
}

/// Renders the config — the platform control the Swift file owned.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the dispatcher hands the config by value"
)]
fn render(config: MultiDatePickerConfig, ctx: &RenderContext<'_>) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);
    let host_view: &PlatformView = &host;

    let label = ctx
        .render(waterui_backend_core::AnyView::new(config.label.clone()))
        .mount(host_view);
    cocoa_ui::view::hide_from_accessibility(label.view());

    let decorated_keys = Rc::new(RefCell::new(
        config
            .decorated
            .snapshot()
            .iter()
            .map(|day| parts_of(*day))
            .collect::<BTreeSet<DateParts>>(),
    ));

    let state = Rc::new(RefCell::new(MultiState {
        #[cfg(target_os = "macos")]
        host: cocoa_ui::view::retain_base(host_view),
        #[cfg(target_os = "macos")]
        mtm,
        label,
        value: config.value.clone(),
        decorated: config.decorated.clone(),
        range: config.range.clone(),
        decorated_keys: Rc::clone(&decorated_keys),
        #[cfg(target_os = "ios")]
        calendar: platform::CalendarView::new(mtm),
        #[cfg(target_os = "ios")]
        decoration_color: Rc::new(RefCell::new(platform::UIColor::secondaryLabelColor())),
        #[cfg(target_os = "macos")]
        picker: platform::DatePicker::new(mtm, platform::DatePickerElements::Date),
        #[cfg(target_os = "macos")]
        toggle: platform::Button::new(mtm),
        #[cfg(target_os = "macos")]
        rows: RefCell::new(Vec::new()),
        #[cfg(target_os = "macos")]
        font: RefCell::new(cocoa_ui::font::system(
            mtm,
            0.0,
            cocoa_ui::font::weight::REGULAR,
        )),
        #[cfg(target_os = "macos")]
        foreground: RefCell::new(platform::NSColor::labelColor()),
    }));

    platform_controls(host_view, &state, &decorated_keys);

    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |view| layout_children(view, &state.borrow())
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        MultiSubView {
            state: Rc::clone(&state),
        },
    );

    // The picker's own date change toggles the day it shows — the shared
    // `toggleCurrentDate` selector on `AppKit`.
    #[cfg(target_os = "macos")]
    let picker_target = state.borrow().picker.install_action({
        let state = Rc::clone(&state);
        move || {
            let current = current_date(&state.borrow());
            apply_selection_toggle(&state.borrow().value, current);
        }
    });

    wire(&mut leaf, &config, &state, ctx);

    #[cfg(target_os = "macos")]
    leaf.keep(picker_target);

    // `syncFromModel` runs once the control is fully wired — as the Swift
    // init's trailing call.
    sync_from_model(&state.borrow());

    leaf.keep(state);
    leaf
}

/// Installs the `multi_date_picker` handler: the composer claim plus the
/// `Native<MultiDatePickerConfig>` claim for hook-produced payloads.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<MultiDatePickerConfig>(render);
    dispatcher.register_view::<MultiDatePicker>(|picker, ctx| {
        let mut config = picker.config();
        // `body()`: an environment hook wins; otherwise the label resolves.
        if let Some(hook) = ctx.env().get::<Hook<MultiDatePickerConfig>>() {
            return ctx.render(hook.apply(ctx.env(), config));
        }
        config.label = config.label.resolve(ctx.env());
        render(config, ctx)
    });
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object.
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

/// The platform weight of a `FontWeight` on the `NSFont`/`UIFont` scale.
#[cfg(target_os = "macos")]
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

/// The platform face a resolved font names — the `Body` resolved font
/// applied to the picker text and selection rows.
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

/// The `AppKit` variant: headroom goes through
/// `NSColor.applyingContentHeadroom` rather than scaled components.
#[cfg(target_os = "macos")]
fn platform_color(color: &waterui::graphics::color::WorkingColor) -> Retained<platform::NSColor> {
    {
        let [red, green, blue, alpha] = color.components;
        cocoa_ui::appkit::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i16, month: i8, day: i8) -> Date {
        Date::new(year, month, day).unwrap()
    }

    #[test]
    fn toggle_adds_sorted_and_removes() {
        let mut selected = vec![day(2025, 3, 10), day(2025, 3, 1)];
        toggle_selection(&mut selected, day(2025, 3, 5));
        assert_eq!(
            selected,
            vec![day(2025, 3, 1), day(2025, 3, 5), day(2025, 3, 10)]
        );
        toggle_selection(&mut selected, day(2025, 3, 5));
        assert_eq!(selected, vec![day(2025, 3, 1), day(2025, 3, 10)]);
    }

    #[test]
    fn can_toggle_honors_the_range() {
        let range = day(2025, 1, 1)..=day(2025, 1, 31);
        assert!(can_toggle(&range, parts_of(day(2025, 1, 1))));
        assert!(can_toggle(&range, parts_of(day(2025, 1, 31))));
        assert!(can_toggle(&range, parts_of(day(2025, 1, 15))));
        assert!(!can_toggle(&range, parts_of(day(2025, 2, 1))));
        assert!(!can_toggle(&range, parts_of(day(2024, 12, 31))));
    }

    #[test]
    fn date_parts_round_trip() {
        let value = day(2026, 9, 29);
        assert_eq!(to_jiff(parts_of(value)), value);
    }
}
