//! The `calendar` leaf: `Calendar` claimed before `body()` expands it,
//! rendered as the label on top of the platform month calendar.
//!
//! On `UIKit` the calendar is a `UICalendarView` with single-date
//! selection. On `AppKit` it is [`cocoa_ui::appkit::CalendarView`], the
//! month calendar drawn like the graphical `NSDatePicker` — the picker
//! itself has no decoration API, so it cannot carry the component.
//!
//! The leaf is generic over the selection a component binds
//! ([`CalendarSelection`]), so every calendar-shaped component shares one
//! realization: selection and decorations push into the control on every
//! change, the range bounds what can be picked, the visible month binds
//! both ways, and the label's text names the control to assistive
//! technology.

#[cfg(target_os = "ios")]
use alloc::collections::BTreeSet;
use alloc::rc::Rc;
use alloc::vec::Vec;
#[cfg(target_os = "ios")]
use core::cell::RefCell;
use core::ops::RangeInclusive;

use cocoa_ui::date::{CalendarSelectionMode, DateParts, YearMonth};
use cocoa_ui::{PlatformView, Rect, Retained};
use waterui::component::form::picker::date::Date;
use waterui::component::label::Label;
#[cfg(target_os = "macos")]
use waterui::reactive::SignalExt;
use waterui::reactive::{Binding, Computed, Signal};
use waterui::resolve::Resolvable;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, Size, StretchAxis, SubView,
    ViewDimensions,
};

use crate::contract::{Mounted, NativeLeaf, RenderContext};

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::{CalendarView, HostView};
    pub(super) use cocoa_ui::objc2_app_kit::NSColor as PlatformColor;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::objc2_ui_kit::UIColor as PlatformColor;
    pub(super) use cocoa_ui::uikit::{CalendarView, HostView};
}

use platform::HostView;

/// The vertical gap between the label and the calendar.
const SPACING: f64 = 8.0;

/// The `jiff` `Date` as the kit's plain parts.
pub(super) fn parts_of(value: Date) -> DateParts {
    DateParts {
        year: i32::from(value.year()),
        month: i32::from(value.month()),
        day: i32::from(value.day()),
    }
}

/// `DateParts` back into `jiff`.
///
/// # Panics
///
/// When the parts are not a valid calendar date.
pub(super) fn to_jiff(parts: DateParts) -> Date {
    Date::new(
        i16::try_from(parts.year).expect("year in range"),
        i8::try_from(parts.month).expect("month in range"),
        i8::try_from(parts.day).expect("day in range"),
    )
    .expect("WaterUI received an invalid calendar date")
}

/// The `NSDate` at midnight of `value`.
///
/// # Panics
///
/// When the Gregorian calendar cannot resolve `value`.
#[cfg(target_os = "ios")]
fn ns_date(value: Date) -> Retained<objc2_foundation::NSDate> {
    cocoa_ui::date::ns_date_from_date_parts(parts_of(value))
        .expect("WaterUI received an invalid calendar date")
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color.
#[cfg(target_os = "ios")]
pub(super) fn platform_color(
    color: &waterui::graphics::color::WorkingColor,
) -> Retained<platform::PlatformColor> {
    let [red, green, blue, alpha] = color.components;
    cocoa_ui::uikit::colors::extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

/// The `AppKit` variant: headroom goes through
/// `NSColor.applyingContentHeadroom` rather than scaled components.
#[cfg(target_os = "macos")]
pub(super) fn platform_color(
    color: &waterui::graphics::color::WorkingColor,
) -> Retained<platform::PlatformColor> {
    let [red, green, blue, alpha] = color.components;
    cocoa_ui::appkit::colors::extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

/// The selection a calendar-shaped component binds, and what a pick does
/// to it.
pub(super) trait CalendarSelection: Clone + 'static {
    /// How the platform control selects.
    const MODE: CalendarSelectionMode;

    /// The selected days.
    fn selected(&self) -> Vec<Date>;

    /// Applies the user's pick of `day`.
    fn pick(&self, day: Date);

    /// Calls `on_change` whenever the selection changes, for the leaf's
    /// lifetime.
    fn watch(&self, leaf: &mut NativeLeaf, on_change: impl Fn() + 'static);
}

#[cfg(feature = "calendar")]
impl CalendarSelection for Binding<Date> {
    const MODE: CalendarSelectionMode = CalendarSelectionMode::Single;

    fn selected(&self) -> Vec<Date> {
        alloc::vec![self.snapshot()]
    }

    fn pick(&self, day: Date) {
        self.set(day);
    }

    fn watch(&self, leaf: &mut NativeLeaf, on_change: impl Fn() + 'static) {
        leaf.watch(self, move |_| on_change());
    }
}

/// Everything a calendar leaf realizes.
pub(super) struct CalendarSpec<S> {
    /// The label above the calendar, already resolved.
    pub(super) label: Label,
    /// The bound selection.
    pub(super) selection: S,
    /// The selectable days.
    pub(super) range: RangeInclusive<Date>,
    /// The days carrying a decoration dot.
    pub(super) decorated: Computed<Vec<Date>>,
    /// The month shown: any day of it reads as that month; navigation
    /// writes the month's first day.
    pub(super) visible_month: Binding<Date>,
}

/// The mounted pieces the layout handler and the watchers share.
struct CalendarLeaf {
    /// The rendered label — the stack's first row.
    label: Mounted,
    /// The platform month calendar.
    #[cfg(target_os = "macos")]
    calendar: Retained<platform::CalendarView>,
    /// The platform month calendar.
    #[cfg(target_os = "ios")]
    calendar: platform::CalendarView,
    /// The decorated days the `UIKit` decoration delegate looks up.
    #[cfg(target_os = "ios")]
    decorated_keys: Rc<RefCell<BTreeSet<DateParts>>>,
    /// The decoration's current `MutedForeground` color.
    #[cfg(target_os = "ios")]
    decoration_color: Rc<RefCell<Retained<platform::PlatformColor>>>,
}

impl CalendarLeaf {
    #[cfg(target_os = "macos")]
    fn calendar_view(&self) -> &PlatformView {
        &self.calendar
    }

    #[cfg(target_os = "ios")]
    fn calendar_view(&self) -> &PlatformView {
        self.calendar.view()
    }

    fn label_size(&self) -> Size {
        self.label.layout().measure(ProposalSize::UNSPECIFIED).size
    }

    /// Pushes the selection and decorations onto the control.
    fn sync(&self, selected: &[Date], decorated: &[Date]) {
        let selected: Vec<DateParts> = selected.iter().map(|day| parts_of(*day)).collect();
        let decorated: Vec<DateParts> = decorated.iter().map(|day| parts_of(*day)).collect();
        #[cfg(target_os = "macos")]
        {
            self.calendar.set_selection(selected);
            self.calendar.set_decorations(decorated);
        }
        #[cfg(target_os = "ios")]
        {
            *self.decorated_keys.borrow_mut() = decorated.iter().copied().collect();
            self.calendar.sync(&selected, &decorated);
        }
    }

    /// Frames the label across the top and the calendar under it.
    fn layout(&self, view: &PlatformView) {
        let bounds = cocoa_ui::view::bounds(view);
        let label_height = f64::from(self.label_size().height);
        cocoa_ui::view::set_frame(
            self.label.view(),
            Rect::new(0.0, 0.0, bounds.size.width, label_height),
        );
        let top = label_height + SPACING;
        #[cfg(target_os = "macos")]
        let frame = {
            let size = self.calendar.intrinsic_size();
            Rect::new(0.0, top, size.width, size.height)
        };
        #[cfg(target_os = "ios")]
        let frame = Rect::new(
            0.0,
            top,
            bounds.size.width,
            (bounds.size.height - top).max(0.0),
        );
        cocoa_ui::view::set_frame(self.calendar_view(), frame);
    }
}

/// The leaf's layout face: label and calendar stacked at their natural
/// sizes, the calendar compressing to the offered width on `UIKit`.
struct CalendarSubView {
    leaf: Rc<CalendarLeaf>,
}

impl core::fmt::Debug for CalendarSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CalendarSubView").finish_non_exhaustive()
    }
}

impl SubView for CalendarSubView {
    fn resolve_extent(
        &self,
        bounds: LayoutRect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let label = self.leaf.label_size();
        // The `AppKit` calendar is fixed-size, as the graphical picker is.
        #[cfg(target_os = "macos")]
        let (width, calendar_height) = {
            let _ = proposal;
            let calendar = self.leaf.calendar.intrinsic_size();
            (f64::from(label.width).max(calendar.width), calendar.height)
        };
        // `UICalendarView` compresses to the offered width, so the leaf
        // never answers wider — otherwise the scroll chain re-measures at
        // the overflow and widens the whole column — and its height is
        // what it needs at that width.
        #[cfg(target_os = "ios")]
        let (width, calendar_height) = {
            let natural = self.leaf.calendar.intrinsic_size().width;
            let mut width = f64::from(label.width).max(natural);
            if let Some(bound) = proposal.width.filter(|w| w.is_finite() && *w > 0.0) {
                width = width.min(f64::from(bound));
            }
            (width, self.leaf.calendar.fitting_size(width).height)
        };
        let height = f64::from(label.height) + SPACING + calendar_height;
        ViewDimensions::new(Size::new(width as f32, height as f32))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// The `AppKit` calendar's colours from the theme tokens.
#[cfg(target_os = "macos")]
fn calendar_colors(env: &waterui::Environment) -> cocoa_ui::appkit::CalendarColors {
    use waterui::theme::color::{
        Accent, Border, Foreground, MutedForeground, SelectionContainer, SelectionForeground,
    };
    cocoa_ui::appkit::CalendarColors {
        foreground: platform_color(&Foreground.resolve(env).snapshot()),
        muted: platform_color(&MutedForeground.resolve(env).snapshot()),
        separator: platform_color(&Border.resolve(env).snapshot()),
        accent: platform_color(&Accent.resolve(env).snapshot()),
        selection: platform_color(&SelectionContainer.resolve(env).snapshot()),
        selection_foreground: platform_color(&SelectionForeground.resolve(env).snapshot()),
    }
}

/// Repaints the `AppKit` calendar whenever a token it draws with changes.
#[cfg(target_os = "macos")]
fn bind_colors(leaf: &mut NativeLeaf, env: &waterui::Environment, calendar: &Rc<CalendarLeaf>) {
    use waterui::theme::color::{
        Accent, Border, Foreground, MutedForeground, SelectionContainer, SelectionForeground,
    };
    for token in [
        Foreground.resolve(env).computed(),
        MutedForeground.resolve(env).computed(),
        Border.resolve(env).computed(),
        Accent.resolve(env).computed(),
        SelectionContainer.resolve(env).computed(),
        SelectionForeground.resolve(env).computed(),
    ] {
        leaf.bind(&token, {
            let env = env.clone();
            let calendar = Rc::clone(calendar);
            move |_| calendar.calendar.set_colors(calendar_colors(&env))
        });
    }
}

/// Names ◀ ● ▶ with the localizable labels the form crate defines, in the
/// environment's locale.
#[cfg(target_os = "macos")]
fn bind_navigation_labels(
    leaf: &mut NativeLeaf,
    env: &waterui::Environment,
    calendar: &Rc<CalendarLeaf>,
) {
    use waterui::component::form::calendar::{
        calendar_next_month_label, calendar_previous_month_label, calendar_selected_month_label,
    };
    let spoken = |mut label: Label| {
        label.resolve_in_place(env);
        label.accessibility_label()
    };
    let previous = spoken(calendar_previous_month_label());
    let selection = spoken(calendar_selected_month_label());
    let next = spoken(calendar_next_month_label());
    let plain = |text: &Computed<waterui::text::styled::StyledStr>| {
        cocoa_ui::text::strip_bidi_controls(&text.snapshot().to_plain())
    };
    let apply = {
        let calendar = Rc::clone(calendar);
        let (previous, selection, next) = (previous.clone(), selection.clone(), next.clone());
        move || {
            calendar
                .calendar
                .set_navigation_labels(cocoa_ui::appkit::CalendarNavigationLabels {
                    previous: plain(&previous),
                    selection: plain(&selection),
                    next: plain(&next),
                });
        }
    };
    let apply = Rc::new(apply);
    for label in [previous, selection, next] {
        let apply = Rc::clone(&apply);
        leaf.bind(&label, move |_| apply());
    }
}

/// Builds the platform calendar for `mode`.
#[cfg(target_os = "macos")]
fn platform_calendar(
    mtm: cocoa_ui::MainThreadMarker,
    mode: CalendarSelectionMode,
) -> Retained<platform::CalendarView> {
    // The labels are replaced from the environment as soon as the leaf
    // binds them.
    platform::CalendarView::new(
        mtm,
        mode,
        cocoa_ui::appkit::CalendarNavigationLabels {
            previous: alloc::string::String::new(),
            selection: alloc::string::String::new(),
            next: alloc::string::String::new(),
        },
    )
}

/// Builds the platform calendar for `mode`.
#[cfg(target_os = "ios")]
fn platform_calendar(
    mtm: cocoa_ui::MainThreadMarker,
    mode: CalendarSelectionMode,
) -> platform::CalendarView {
    platform::CalendarView::new(mtm, mode)
}

/// Renders a calendar-shaped component: the label over the platform month
/// calendar, bound to `spec`.
pub(super) fn render_calendar<S: CalendarSelection>(
    spec: CalendarSpec<S>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let CalendarSpec {
        label,
        selection,
        range,
        decorated,
        visible_month,
    } = spec;
    let mtm = ctx.mtm();
    let env = ctx.env();
    let host = HostView::new(mtm, Rect::ZERO);
    let host_view: &PlatformView = &host;

    let accessibility_label = label.accessibility_label();
    let mounted_label = ctx
        .render(waterui_backend_core::AnyView::new(label))
        .mount(host_view);
    cocoa_ui::view::hide_from_accessibility(mounted_label.view());

    let calendar = Rc::new(CalendarLeaf {
        label: mounted_label,
        calendar: platform_calendar(mtm, S::MODE),
        #[cfg(target_os = "ios")]
        decorated_keys: Rc::new(RefCell::new(BTreeSet::new())),
        #[cfg(target_os = "ios")]
        decoration_color: Rc::new(RefCell::new(platform::PlatformColor::secondaryLabelColor())),
    });
    cocoa_ui::view::add_subview(host_view, calendar.calendar_view());
    install_handlers(&calendar, &selection, &range, &visible_month);

    host.set_layout_handler({
        let calendar = Rc::clone(&calendar);
        move |view| calendar.layout(view)
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        CalendarSubView {
            leaf: Rc::clone(&calendar),
        },
    );

    // Selection and decorations push into the control on every change.
    let sync = Rc::new({
        let calendar = Rc::clone(&calendar);
        let selection = selection.clone();
        let decorated = decorated.clone();
        move || calendar.sync(&selection.selected(), &decorated.snapshot())
    });
    selection.watch(&mut leaf, {
        let sync = Rc::clone(&sync);
        move || sync()
    });
    leaf.watch(&decorated, {
        let sync = Rc::clone(&sync);
        move |_| sync()
    });
    leaf.watch(&visible_month, {
        let calendar = Rc::clone(&calendar);
        move |context| {
            calendar
                .calendar
                .set_visible_month(YearMonth::of(parts_of(*context.value())));
        }
    });

    // The environment's locale formats the calendar.
    leaf.bind(&waterui_locale::locale_binding(env), {
        let calendar = Rc::clone(&calendar);
        move |locale: waterui_locale::Locale| {
            calendar.calendar.set_locale(&locale.canonical_tag());
        }
    });

    bind_appearance(&mut leaf, env, &calendar, &decorated);

    // The label's semantic text names the control.
    leaf.bind(&accessibility_label, {
        let calendar = Rc::clone(&calendar);
        move |styled| {
            let text = cocoa_ui::text::strip_bidi_controls(&styled.to_plain());
            calendar.calendar.set_accessibility_label(Some(&text));
        }
    });

    sync();
    leaf.keep(sync);
    leaf.keep(calendar);
    leaf
}

/// Wires the range, the picks and the month navigation into the control.
fn install_handlers<S: CalendarSelection>(
    calendar: &Rc<CalendarLeaf>,
    selection: &S,
    range: &RangeInclusive<Date>,
    visible_month: &Binding<Date>,
) {
    #[cfg(target_os = "macos")]
    {
        calendar
            .calendar
            .set_range(parts_of(*range.start())..=parts_of(*range.end()));
        calendar.calendar.on_pick({
            let selection = selection.clone();
            move |parts| selection.pick(to_jiff(parts))
        });
    }
    #[cfg(target_os = "ios")]
    {
        calendar
            .calendar
            .set_available_range(&cocoa_ui::date::interval(
                &ns_date(*range.start()),
                &ns_date(*range.end()),
            ));
        calendar.calendar.set_can_select({
            let range = range.clone();
            move |parts| range.contains(&to_jiff(parts))
        });
        calendar.calendar.on_pick({
            let selection = selection.clone();
            move |parts| selection.pick(to_jiff(parts))
        });
        calendar.calendar.set_decoration({
            let keys = Rc::clone(&calendar.decorated_keys);
            let color = Rc::clone(&calendar.decoration_color);
            move |parts| {
                keys.borrow()
                    .contains(&parts)
                    .then(|| color.borrow().clone())
            }
        });
    }
    calendar
        .calendar
        .set_visible_month(YearMonth::of(parts_of(visible_month.snapshot())));
    calendar.calendar.on_visible_month_change({
        let visible_month = visible_month.clone();
        move |month| visible_month.set(to_jiff(month.first_day()))
    });
}

/// Binds the theme to the control: the `AppKit` calendar's colours and
/// spoken navigation labels, the `UIKit` decoration colour.
fn bind_appearance(
    leaf: &mut NativeLeaf,
    env: &waterui::Environment,
    calendar: &Rc<CalendarLeaf>,
    decorated: &Computed<Vec<Date>>,
) {
    #[cfg(target_os = "macos")]
    {
        let _ = decorated;
        bind_colors(leaf, env, calendar);
        bind_navigation_labels(leaf, env, calendar);
    }
    #[cfg(target_os = "ios")]
    {
        // `Accent` tints the selection and the navigation.
        let accent = waterui::theme::color::Accent.resolve(env);
        leaf.bind(&accent, {
            let calendar = Rc::clone(calendar);
            move |color: waterui::graphics::color::WorkingColor| {
                cocoa_ui::view::set_tint_color(
                    calendar.calendar_view(),
                    Some(&platform_color(&color)),
                );
            }
        });
        // `MutedForeground` recolors the decoration dots.
        let muted = waterui::theme::color::MutedForeground.resolve(env);
        leaf.bind(&muted, {
            let calendar = Rc::clone(calendar);
            let decorated = decorated.clone();
            move |color: waterui::graphics::color::WorkingColor| {
                *calendar.decoration_color.borrow_mut() = platform_color(&color);
                let days: Vec<DateParts> = decorated
                    .snapshot()
                    .iter()
                    .map(|day| parts_of(*day))
                    .collect();
                calendar.calendar.reload_decorations(&days);
            }
        });
    }
}

#[cfg(feature = "calendar")]
pub use component::install;

/// The `Calendar` component's handler.
#[cfg(feature = "calendar")]
mod component {
    use alloc::collections::BTreeSet;
    use alloc::vec::Vec;

    use waterui::component::form::calendar::{Calendar, CalendarConfig};
    use waterui::component::form::picker::date::Date;
    use waterui::reactive::SignalExt;
    use waterui::view::{ConfigurableView, Hook};

    use super::{CalendarSpec, render_calendar};
    use crate::contract::{NativeLeaf, RenderContext};
    use crate::dispatch::Dispatcher;

    /// Renders `Calendar`'s config.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the dispatcher hands the config by value"
    )]
    fn render(config: CalendarConfig, ctx: &RenderContext<'_>) -> NativeLeaf {
        let decorated = config
            .decorated()
            .map(|days: BTreeSet<Date>| days.into_iter().collect::<Vec<_>>())
            .computed();
        render_calendar(
            CalendarSpec {
                label: config.label().clone(),
                selection: config.value().clone(),
                range: config.range().clone(),
                decorated,
                visible_month: config.visible_month().clone(),
            },
            ctx,
        )
    }

    /// Installs the `calendar` handler: the composer claim plus the
    /// `Native<CalendarConfig>` claim for hook-produced payloads.
    pub fn install(dispatcher: &mut Dispatcher) {
        dispatcher.register_native::<CalendarConfig>(render);
        dispatcher.register_view::<Calendar>(|calendar, ctx| {
            let mut config = calendar.config();
            // `body()`: an environment hook wins; otherwise the label resolves.
            if let Some(hook) = ctx.env().get::<Hook<CalendarConfig>>() {
                return ctx.render(hook.apply(ctx.env(), config));
            }
            config.label_mut().resolve_in_place(ctx.env());
            render(config, ctx)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_parts_round_trip() {
        let value = Date::new(2026, 9, 29).unwrap();
        assert_eq!(to_jiff(parts_of(value)), value);
    }
}
