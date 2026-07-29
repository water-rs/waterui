use waterui::Identifiable;
use waterui::app::App;
use waterui::background::Material;
use waterui::component::list::{List, ListItem};
use waterui::prelude::theme_color::{Foreground, MutedForeground};
use waterui::prelude::*;
use waterui::shape::RoundedRectangle;
use waterui::widget::condition::when;
use waterui_icons_material_icon as mdi;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum SidebarDestination {
    Today,
    Scheduled,
    All,
    Flagged,
    Completed,
}

impl SidebarDestination {
    const fn title(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Scheduled => "Scheduled",
            Self::All => "All",
            Self::Flagged => "Flagged",
            Self::Completed => "Completed",
        }
    }

    fn icon(self) -> waterui_icons_material_icon::Svg {
        match self {
            Self::Today => mdi::calendar_today(),
            Self::Scheduled => mdi::calendar_clock(),
            Self::All => mdi::inbox(),
            Self::Flagged => mdi::flag(),
            Self::Completed => mdi::check_circle(),
        }
    }

    const fn icon_color(self) -> Srgb {
        match self {
            Self::Today => Srgb::from_hex("#4A84F6"),
            Self::Scheduled => Srgb::from_hex("#F5B84A"),
            Self::All => Srgb::from_hex("#8F8F96"),
            Self::Flagged => Srgb::from_hex("#F28A34"),
            Self::Completed => Srgb::from_hex("#30BA61"),
        }
    }
}

#[derive(Clone, Identifiable)]
struct SidebarRow {
    #[id]
    id: u32,
    dest: SidebarDestination,
    count: i32,
}

#[derive(Clone, Identifiable)]
struct ReminderRow {
    #[id]
    id: u64,
    title: &'static str,
    subtitle: Option<&'static str>,
    flagged: bool,
}

fn sidebar_rows() -> [SidebarRow; 5] {
    [
        SidebarRow {
            id: 1,
            dest: SidebarDestination::Today,
            count: 6,
        },
        SidebarRow {
            id: 2,
            dest: SidebarDestination::Scheduled,
            count: 2,
        },
        SidebarRow {
            id: 3,
            dest: SidebarDestination::All,
            count: 18,
        },
        SidebarRow {
            id: 4,
            dest: SidebarDestination::Flagged,
            count: 1,
        },
        SidebarRow {
            id: 5,
            dest: SidebarDestination::Completed,
            count: 12,
        },
    ]
}

fn reminders_for(dest: SidebarDestination) -> (&'static [ReminderRow], &'static [ReminderRow]) {
    match dest {
        SidebarDestination::Today => (
            &[
                ReminderRow {
                    id: 1,
                    title: "Call dentist",
                    subtitle: Some("2:00 PM"),
                    flagged: false,
                },
                ReminderRow {
                    id: 2,
                    title: "Review navigation parity worktree",
                    subtitle: Some("Before lunch"),
                    flagged: true,
                },
            ],
            &[ReminderRow {
                id: 3,
                title: "Pick up package",
                subtitle: Some("Tomorrow 10:00 AM"),
                flagged: false,
            }],
        ),
        SidebarDestination::Scheduled => (
            &[ReminderRow {
                id: 4,
                title: "Book flight",
                subtitle: Some("Fri"),
                flagged: false,
            }],
            &[ReminderRow {
                id: 5,
                title: "Pay utilities",
                subtitle: Some("Next week"),
                flagged: false,
            }],
        ),
        SidebarDestination::All => (
            &[
                ReminderRow {
                    id: 6,
                    title: "Plan weekend",
                    subtitle: None,
                    flagged: false,
                },
                ReminderRow {
                    id: 7,
                    title: "Update roadmap",
                    subtitle: None,
                    flagged: true,
                },
            ],
            &[ReminderRow {
                id: 8,
                title: "Refactor split navigation chrome",
                subtitle: Some("Cross-platform"),
                flagged: false,
            }],
        ),
        SidebarDestination::Flagged => (
            &[ReminderRow {
                id: 9,
                title: "Prepare demo",
                subtitle: Some("High priority"),
                flagged: true,
            }],
            &[],
        ),
        SidebarDestination::Completed => (
            &[
                ReminderRow {
                    id: 10,
                    title: "Submit timesheet",
                    subtitle: None,
                    flagged: false,
                },
                ReminderRow {
                    id: 11,
                    title: "Clean inbox",
                    subtitle: None,
                    flagged: false,
                },
            ],
            &[],
        ),
    }
}

fn normalized_search_query(search: &Str) -> Option<String> {
    let query = search.as_str().trim();
    if query.is_empty() {
        None
    } else {
        Some(query.to_ascii_lowercase())
    }
}

fn reminder_matches_query(reminder: &ReminderRow, query: &str) -> bool {
    let title = reminder.title.to_ascii_lowercase();
    let title_matches = title.as_str().contains(query);

    let subtitle_matches = reminder.subtitle.is_some_and(|subtitle| {
        let subtitle = subtitle.to_ascii_lowercase();
        subtitle.as_str().contains(query)
    });

    title_matches || subtitle_matches
}

fn matching_reminder_count(rows: &[ReminderRow], normalized_query: &str) -> usize {
    rows.iter()
        .filter(|row| reminder_matches_query(row, normalized_query))
        .count()
}

#[preview]
pub fn demo() -> impl View {
    let selection = Binding::container(Some(SidebarDestination::Today));
    let search = Binding::container(Str::default());

    NavigationSplitView::new(
        &selection,
        {
            let selection = selection.clone();
            let search = search.clone();
            move || sidebar(selection.clone(), search.clone())
        },
        {
            let search = search.clone();
            move |dest| detail_view(dest, search.clone())
        },
    )
    .sidebar_width(ColumnWidth::new(240.0, 300.0, 420.0))
    .placeholder(placeholder_view)
}

fn sidebar(selection: Binding<Option<SidebarDestination>>, search: Binding<Str>) -> impl View {
    let rows = sidebar_rows();
    let query = search.clone().map(|value| normalized_search_query(&value));

    vstack((
        vstack((
            text("Reminders").title().bold().foreground(Foreground),
            text!("Search: {search}")
                .caption()
                .foreground(MutedForeground),
            text("My Lists").caption().foreground(MutedForeground),
        ))
        .spacing(10.0)
        .padding_with(EdgeInsets::all(16.0)),
        List::for_each(rows, move |row| {
            let dest = row.dest;
            let is_selected = selection.clone().map(move |current| current == Some(dest));
            let selection_for_action = selection.clone();
            let query = query.clone();
            let bg = is_selected
                .select(
                    Srgb::WHITE.with_opacity(0.14),
                    Srgb::WHITE.with_opacity(0.0),
                )
                .computed();
            let count = query.map(move |query| {
                if let Some(query) = query.as_deref() {
                    let (today_rows, upcoming_rows) = reminders_for(dest);
                    matching_reminder_count(today_rows, query) as i32
                        + matching_reminder_count(upcoming_rows, query) as i32
                } else {
                    row.count
                }
            });

            ListItem::new(
                hstack((
                    dest.icon().size(18.0, 18.0).tint(dest.icon_color()),
                    text(dest.title()).body().foreground(Foreground),
                    spacer(),
                    text!("{count}").caption().foreground(MutedForeground),
                ))
                .padding_with(EdgeInsets::symmetric(10.0, 14.0))
                .background(signal_color(bg))
                .clip(RoundedRectangle::new(10.0))
                .on_tap({
                    let selection_for_action = selection_for_action.clone();
                    move || selection_for_action.set(Some(dest))
                }),
            )
        }),
    ))
    .width(300.0)
    .background(Material::Thick)
}

fn detail_view(dest: SidebarDestination, search: Binding<Str>) -> NavigationView {
    let (today_rows, upcoming_rows) = reminders_for(dest);

    vstack((
        content_header(dest),
        Divider,
        reminder_section("Today", today_rows, search.clone()),
        reminder_section("Upcoming", upcoming_rows, search.clone()),
    ))
    .background(Material::Regular)
    .title(dest.title())
    .searchable(&search, "Search reminders")
    .navigation_toolbar(NavigationToolbar::new(vec![NavigationToolbarItem::new(
        NavigationToolbarPlacement::PrimaryAction,
        button(label("").icon(mdi::plus()))
            .style(ButtonStyle::Borderless)
            .action(|| {}),
    )]))
}

fn placeholder_view() -> impl View {
    vstack((
        text("Select a list").title().foreground(Foreground),
        text("Choose one of your reminder collections from the sidebar.")
            .body()
            .foreground(MutedForeground),
    ))
    .spacing(10.0)
    .background(Material::Regular)
}

fn content_header(dest: SidebarDestination) -> impl View {
    vstack((
        text(dest.title()).title().bold().foreground(Foreground),
        text("Friday, February 6")
            .caption()
            .foreground(MutedForeground),
    ))
    .spacing(6.0)
    .padding_with(EdgeInsets::new(14.0, 18.0, 12.0, 18.0))
}

fn reminder_visible(search: Binding<Str>, row: ReminderRow) -> Computed<bool> {
    search
        .map(move |query| {
            normalized_search_query(&query)
                .as_deref()
                .is_none_or(|query| reminder_matches_query(&row, query))
        })
        .computed()
}

fn section_visible(search: Binding<Str>, rows: &'static [ReminderRow]) -> Computed<bool> {
    search
        .map(move |query| {
            let normalized_query = normalized_search_query(&query);
            match normalized_query.as_deref() {
                Some(query) => rows
                    .iter()
                    .any(|reminder| reminder_matches_query(reminder, query)),
                None => !rows.is_empty(),
            }
        })
        .computed()
}

fn reminder_section(
    title: &'static str,
    rows: &'static [ReminderRow],
    search: Binding<Str>,
) -> impl View {
    let visible = section_visible(search.clone(), rows);
    vstack((
        text(title)
            .caption()
            .bold()
            .foreground(MutedForeground)
            .padding_with(EdgeInsets::new(8.0, 18.0, 0.0, 18.0)),
        List::for_each(rows, move |row| {
            let visible = reminder_visible(search.clone(), row.clone());
            ListItem::new(
                hstack((
                    mdi::circle_outline()
                        .size(16.0, 16.0)
                        .foreground(MutedForeground),
                    vstack((
                        text(row.title).body().foreground(Foreground),
                        row.subtitle
                            .map(|subtitle| text(subtitle).caption().foreground(MutedForeground)),
                    ))
                    .spacing(2.0),
                    spacer(),
                    when(row.flagged, || {
                        mdi::flag().size(12.0, 12.0).tint(Srgb::from_hex("#F28A34"))
                    })
                    .otherwise(|| spacer().width(12.0)),
                ))
                .padding_with(EdgeInsets::symmetric(10.0, 18.0))
                .visible(visible),
            )
        }),
    ))
    .visible(visible)
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
