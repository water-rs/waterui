//! The `calendar` leaf: `Calendar` claimed before `body()` expands it,
//! rendered as the label on top of the platform month calendar.
//!
//! On `UIKit` the calendar is a `UICalendarView` with single-date
//! selection. On `AppKit` it is [`cocoa_ui::appkit::CalendarView`], the
//! month calendar composed of `AppKit` controls like the graphical
//! `NSDatePicker` — the picker itself has no decoration API, so it cannot
//! carry the component.
//!
//! The leaf is generic over the selection a component binds
//! ([`CalendarSelection`]), so every calendar-shaped component shares one
//! realization. Each signal updates only what it drives: the selection the
//! selected days, the decorated days the dots, the visible month the shown
//! month (and navigation writes it back), each theme token its own colour,
//! and the label's text the calendar's accessible name. The range bounds
//! what can be picked and shown.

use alloc::collections::BTreeSet;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::ops::RangeInclusive;

use cocoa_ui::date::{CalendarSelectionMode, DateParts, YearMonth};
use cocoa_ui::{PlatformView, Rect};
use waterui::component::form::picker::date::Date;
use waterui::component::label::Label;
use waterui::reactive::{Binding, Computed, Signal};
#[cfg(target_os = "ios")]
use waterui::resolve::Resolvable;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, Size, StretchAxis, SubView,
    ViewDimensions,
};

use crate::contract::{Mounted, NativeLeaf, RenderContext};
use crate::proposal;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::toggle::LABEL_SPACING;
    pub(super) use cocoa_ui::appkit::{CalendarView, HostView};
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::toggle::LABEL_SPACING;
    pub(super) use cocoa_ui::uikit::{CalendarView, HostView};
}

use platform::{HostView, LABEL_SPACING};

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
/// When the parts are not a valid calendar date — impossible for control
/// output: the platform calendar only reports the date of a real day.
pub(super) fn to_jiff(parts: DateParts) -> Date {
    Date::new(
        i16::try_from(parts.year).expect("the calendar reports a year that fits i16"),
        i8::try_from(parts.month).expect("the calendar reports a month that fits i8"),
        i8::try_from(parts.day).expect("the calendar reports a day that fits i8"),
    )
    .expect("the calendar reports the parts of a real Gregorian date")
}

/// The `NSDate` at midnight of `value`. `None`, logged, when the Gregorian
/// calendar cannot resolve `value`: the caller skips the step that needed it.
#[cfg(target_os = "ios")]
fn ns_date(value: Date) -> Option<cocoa_ui::Retained<objc2_foundation::NSDate>> {
    let date = cocoa_ui::date::ns_date_from_date_parts(parts_of(value));
    if date.is_none() {
        tracing::error!(
            "the Gregorian calendar cannot resolve the calendar date {value}; \
             the date is not applied"
        );
    }
    date
}

/// The selection a calendar-shaped component binds, and what a pick does
/// to it. The leaf watches the selection as the signal it is.
pub(super) trait CalendarSelection: Signal + Clone + 'static {
    /// How the platform control selects.
    const MODE: CalendarSelectionMode;

    /// The selected days in `value`, the selection's current value.
    fn selected(value: &Self::Output) -> Vec<Date>;

    /// Applies the user's pick of `day`.
    fn pick(&self, day: Date);
}

#[cfg(feature = "calendar")]
impl CalendarSelection for Binding<Date> {
    const MODE: CalendarSelectionMode = CalendarSelectionMode::Single;

    fn selected(value: &Date) -> Vec<Date> {
        alloc::vec![*value]
    }

    fn pick(&self, day: Date) {
        self.set(day);
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
    pub(super) decorated: Computed<BTreeSet<Date>>,
    /// The month shown: any day of it reads as that month; navigation
    /// writes the month's first day.
    pub(super) visible_month: Binding<Date>,
}

/// The mounted pieces the layout handler and the watchers share.
struct CalendarLeaf {
    /// The rendered label — the stack's first row.
    label: Mounted,
    /// The platform month calendar.
    calendar: platform::CalendarView,
}

impl CalendarLeaf {
    fn calendar_view(&self) -> &PlatformView {
        self.calendar.view()
    }

    /// The label's size when it may wrap within `width`.
    fn label_size(&self, width: Option<f32>) -> Size {
        self.label
            .layout()
            .measure(ProposalSize::new(width, None))
            .size
    }

    /// Frames the label across the top of `view`'s `response` rectangle
    /// and the calendar under it, at the leading edge.
    fn layout(&self, view: &PlatformView, response: Rect) {
        let width = response.size.width;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the layout contract is f32; view widths always fit"
        )]
        let offer = Some(width as f32);
        let label_height = f64::from(self.label_size(offer).height);
        proposal::place_leading(
            self.label.view(),
            Rect::new(0.0, 0.0, width, label_height).translated(response.origin),
            ProposalSize::new(offer, None),
            cocoa_ui::view::is_right_to_left(view),
        );
        let top = label_height + LABEL_SPACING;
        // AppKit's calendar has a fixed size; UIKit's fills the rest.
        #[cfg(target_os = "macos")]
        let (calendar_width, calendar_height) = {
            let size = self.calendar.intrinsic_size();
            (size.width, size.height)
        };
        #[cfg(target_os = "ios")]
        let (calendar_width, calendar_height) = (width, (response.size.height - top).max(0.0));
        let x = if cocoa_ui::view::is_right_to_left(view) {
            width - calendar_width
        } else {
            0.0
        };
        cocoa_ui::view::set_frame(
            self.calendar_view(),
            Rect::new(x, top, calendar_width, calendar_height).translated(response.origin),
        );
    }
}

/// The leaf's layout face: the label, wrapping within the leaf's width,
/// over the calendar.
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
        let offered = proposal.width.filter(|w| w.is_finite() && *w > 0.0);
        let natural_label = f64::from(self.leaf.label_size(None).width);
        // The `AppKit` calendar is fixed-size, as the graphical picker is;
        // the label takes the offered width beyond it.
        #[cfg(target_os = "macos")]
        let (width, calendar_height) = {
            let calendar = self.leaf.calendar.intrinsic_size();
            let label = offered.map_or(natural_label, |w| natural_label.min(f64::from(w)));
            (label.max(calendar.width), calendar.height)
        };
        // `UICalendarView` compresses to the offered width, so the leaf
        // never answers wider — otherwise the scroll chain re-measures at
        // the overflow and widens the whole column — and its height is
        // what it needs at that width.
        #[cfg(target_os = "ios")]
        let (width, calendar_height) = {
            let natural = natural_label.max(self.leaf.calendar.intrinsic_size().width);
            let width = offered.map_or(natural, |w| natural.min(f64::from(w)));
            (width, self.leaf.calendar.fitting_size(width).height)
        };
        let label_height = f64::from(self.leaf.label_size(Some(width as f32)).height);
        let height = label_height + LABEL_SPACING + calendar_height;
        ViewDimensions::new(Size::new(width as f32, height as f32))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// The days of `days` as the kit's parts.
fn parts_set(days: &BTreeSet<Date>) -> impl Iterator<Item = DateParts> + '_ {
    days.iter().map(|day| parts_of(*day))
}

/// The calendar's accessible name: the label's text without bidi marks.
fn plain(text: &waterui::text::styled::StyledStr) -> alloc::string::String {
    cocoa_ui::text::strip_bidi_controls(&text.to_plain())
}

/// The `AppKit` calendar's colours and spoken navigation labels, read from
/// the environment, and the watchers that push each change on its own.
#[cfg(target_os = "macos")]
mod appearance {
    use alloc::rc::Rc;

    use cocoa_ui::appkit::{
        CalendarColorRole, CalendarColors, CalendarNavigation, CalendarNavigationLabels,
    };
    use waterui::component::form::calendar::{
        calendar_next_month_label, calendar_previous_month_label, calendar_selected_month_label,
    };
    use waterui::component::label::Label;
    use waterui::graphics::color::WorkingColor;
    use waterui::reactive::{Computed, Signal, SignalExt};
    use waterui::resolve::Resolvable;
    use waterui::text::styled::StyledStr;
    use waterui::theme::color::{
        Accent, Border, DisabledForeground, Foreground, MutedForeground, SelectionContainer,
        SelectionForeground, TertiaryForeground, UnemphasizedSelection,
    };

    use super::{CalendarLeaf, plain};
    use crate::contract::NativeLeaf;
    use crate::theme::{platform_color, token_color};

    /// Every theme token the calendar draws with, by role.
    fn tokens(env: &waterui::Environment) -> [(CalendarColorRole, Computed<WorkingColor>); 9] {
        [
            (
                CalendarColorRole::Foreground,
                Foreground.resolve(env).computed(),
            ),
            (
                CalendarColorRole::Muted,
                MutedForeground.resolve(env).computed(),
            ),
            (
                CalendarColorRole::Tertiary,
                TertiaryForeground.resolve(env).computed(),
            ),
            (
                CalendarColorRole::Disabled,
                DisabledForeground.resolve(env).computed(),
            ),
            (CalendarColorRole::Separator, Border.resolve(env).computed()),
            (CalendarColorRole::Accent, Accent.resolve(env).computed()),
            (
                CalendarColorRole::Selection,
                SelectionContainer.resolve(env).computed(),
            ),
            (
                CalendarColorRole::SelectionForeground,
                SelectionForeground.resolve(env).computed(),
            ),
            (
                CalendarColorRole::UnemphasizedSelection,
                UnemphasizedSelection.resolve(env).computed(),
            ),
        ]
    }

    /// The platform colour a role's token draws as: the text tokens as
    /// `AppKit`'s dynamic label colours when they are the theme's own.
    fn color_of(
        env: &waterui::Environment,
        role: CalendarColorRole,
        token: &Computed<WorkingColor>,
    ) -> cocoa_ui::Retained<cocoa_ui::objc2_app_kit::NSColor> {
        match role {
            CalendarColorRole::Foreground
            | CalendarColorRole::Muted
            | CalendarColorRole::Tertiary
            | CalendarColorRole::Disabled => token_color(env, token),
            _ => platform_color(&token.snapshot()),
        }
    }

    /// The colours the calendar starts with.
    pub(super) fn colors(env: &waterui::Environment) -> CalendarColors {
        let [
            foreground,
            muted,
            tertiary,
            disabled,
            separator,
            accent,
            selection,
            selection_foreground,
            unemphasized_selection,
        ] = tokens(env).map(|(role, token)| color_of(env, role, &token));
        CalendarColors {
            foreground,
            muted,
            tertiary,
            disabled,
            separator,
            accent,
            selection,
            selection_foreground,
            unemphasized_selection,
        }
    }

    /// Pushes each token's changes to its role alone.
    pub(super) fn watch_colors(
        leaf: &mut NativeLeaf,
        env: &waterui::Environment,
        calendar: &Rc<CalendarLeaf>,
    ) {
        for (role, token) in tokens(env) {
            let env = env.clone();
            let calendar = Rc::clone(calendar);
            let watched = token.clone();
            leaf.watch(&watched, move |_| {
                calendar
                    .calendar
                    .set_color(role, &color_of(&env, role, &token));
            });
        }
    }

    /// The spoken text of each navigation button, in the environment's
    /// locale.
    fn labels(env: &waterui::Environment) -> [(CalendarNavigation, Computed<StyledStr>); 3] {
        let spoken = |mut label: Label| {
            label.resolve_in_place(env);
            label.accessibility_label()
        };
        [
            (
                CalendarNavigation::Previous,
                spoken(calendar_previous_month_label()),
            ),
            (
                CalendarNavigation::Selection,
                spoken(calendar_selected_month_label()),
            ),
            (
                CalendarNavigation::Next,
                spoken(calendar_next_month_label()),
            ),
        ]
    }

    /// The navigation labels the calendar starts with.
    pub(super) fn navigation_labels(env: &waterui::Environment) -> CalendarNavigationLabels {
        let [previous, selection, next] = labels(env).map(|(_, text)| plain(&text.snapshot()));
        CalendarNavigationLabels {
            previous,
            selection,
            next,
        }
    }

    /// Pushes each navigation label's changes to its button alone.
    pub(super) fn watch_navigation_labels(
        leaf: &mut NativeLeaf,
        env: &waterui::Environment,
        calendar: &Rc<CalendarLeaf>,
    ) {
        for (button, text) in labels(env) {
            let calendar = Rc::clone(calendar);
            leaf.watch(&text, move |change| {
                calendar
                    .calendar
                    .set_navigation_label(button, &plain(change.value()));
            });
        }
    }
}

/// Builds the platform calendar.
#[cfg(target_os = "macos")]
fn platform_calendar(
    ctx: &RenderContext<'_>,
    mode: CalendarSelectionMode,
    range: &RangeInclusive<Date>,
    month: YearMonth,
    locale: &str,
) -> platform::CalendarView {
    platform::CalendarView::new(
        ctx.mtm(),
        mode,
        parts_of(*range.start())..=parts_of(*range.end()),
        month,
        locale,
        appearance::colors(ctx.env()),
        &appearance::navigation_labels(ctx.env()),
    )
}

/// Builds the platform calendar.
#[cfg(target_os = "ios")]
fn platform_calendar(
    ctx: &RenderContext<'_>,
    mode: CalendarSelectionMode,
    range: &RangeInclusive<Date>,
    month: YearMonth,
    locale: &str,
) -> platform::CalendarView {
    let calendar = platform::CalendarView::new(ctx.mtm(), mode, locale);
    if let (Some(start), Some(end)) = (ns_date(*range.start()), ns_date(*range.end())) {
        calendar.set_available_range(&cocoa_ui::date::interval(&start, &end));
    }
    calendar.set_can_select({
        let range = range.clone();
        move |parts| range.contains(&to_jiff(parts))
    });
    calendar.set_visible_month(month);
    calendar
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
    let env = ctx.env();
    let host = HostView::new(ctx.mtm(), Rect::ZERO);
    let host_view: &PlatformView = &host;

    let accessibility_label = label.accessibility_label();
    let mounted_label = ctx
        .render(waterui_backend_core::AnyView::new(label))
        .mount(host_view);
    cocoa_ui::view::hide_from_accessibility(mounted_label.view());

    let locale = waterui_locale::locale_binding(env);
    let month = YearMonth::of(parts_of(visible_month.snapshot()));
    let tag = locale.snapshot().canonical_tag();
    let platform = platform_calendar(ctx, S::MODE, &range, month, &tag);
    let calendar = Rc::new(CalendarLeaf {
        label: mounted_label,
        calendar: platform,
    });
    cocoa_ui::view::add_subview(host_view, calendar.calendar_view());

    host.set_layout_handler({
        let calendar = Rc::clone(&calendar);
        move |view| calendar.layout(view, proposal::response(view))
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        CalendarSubView {
            leaf: Rc::clone(&calendar),
        },
    );

    calendar.calendar.on_pick({
        let selection = selection.clone();
        move |parts| selection.pick(to_jiff(parts))
    });
    calendar.calendar.on_visible_month_change({
        let visible_month = visible_month.clone();
        move |month| visible_month.set(to_jiff(month.first_day()))
    });

    // Each signal pushes its own changes.
    leaf.bind(&selection, {
        let calendar = Rc::clone(&calendar);
        move |value| {
            let selected: Vec<DateParts> = S::selected(&value).into_iter().map(parts_of).collect();
            calendar.calendar.set_selection(&selected);
        }
    });
    leaf.bind(&decorated, {
        let calendar = Rc::clone(&calendar);
        move |days: BTreeSet<Date>| calendar.calendar.set_decorations(parts_set(&days))
    });
    leaf.watch(&visible_month, {
        let calendar = Rc::clone(&calendar);
        move |change| {
            calendar
                .calendar
                .set_visible_month(YearMonth::of(parts_of(*change.value())));
        }
    });
    leaf.watch(&locale, {
        let calendar = Rc::clone(&calendar);
        move |change| {
            calendar
                .calendar
                .set_locale(&change.value().canonical_tag());
        }
    });

    #[cfg(target_os = "macos")]
    {
        appearance::watch_colors(&mut leaf, env, &calendar);
        appearance::watch_navigation_labels(&mut leaf, env, &calendar);
    }
    // `Accent` tints the navigation, as on every `UIKit` control.
    #[cfg(target_os = "ios")]
    leaf.bind(&waterui::theme::color::Accent.resolve(env), {
        let calendar = Rc::clone(&calendar);
        move |color: waterui::graphics::color::WorkingColor| {
            cocoa_ui::view::set_tint_color(
                calendar.calendar_view(),
                Some(&crate::theme::platform_color(&color)),
            );
        }
    });

    // The label's semantic text names the calendar.
    leaf.bind(&accessibility_label, {
        let calendar = Rc::clone(&calendar);
        move |styled| {
            calendar
                .calendar
                .set_accessibility_label(Some(&plain(&styled)));
        }
    });

    leaf.keep(calendar);
    leaf
}

/// `Calendar`'s handler.
#[cfg(feature = "calendar")]
mod component {
    use waterui::component::form::calendar::{Calendar, CalendarConfig};
    use waterui::view::{ConfigurableView, Hook};

    use super::{CalendarSpec, NativeLeaf, RenderContext, render_calendar};
    use crate::dispatch::Dispatcher;

    /// Renders `Calendar`'s config.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the dispatcher hands the config by value"
    )]
    fn render(config: CalendarConfig, ctx: &RenderContext<'_>) -> NativeLeaf {
        render_calendar(
            CalendarSpec {
                label: config.label().clone(),
                selection: config.value().clone(),
                range: config.range().clone(),
                decorated: config.decorated().clone(),
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
            // `body()`: an environment hook wins; otherwise the label
            // resolves.
            if let Some(hook) = ctx.env().get::<Hook<CalendarConfig>>() {
                return ctx.render(hook.apply(ctx.env(), config));
            }
            config.label_mut().resolve_in_place(ctx.env());
            render(config, ctx)
        });
    }
}

#[cfg(feature = "calendar")]
pub use component::install;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_parts_round_trip() {
        let value = Date::new(2026, 9, 29).unwrap();
        assert_eq!(to_jiff(parts_of(value)), value);
    }
}
