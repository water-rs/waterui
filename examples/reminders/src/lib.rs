use waterui::Identifiable;
use waterui::app::App;
use waterui::background::Material;
use waterui::component::list::{List, ListItem};
use waterui::prelude::theme_color::{Foreground, MutedForeground};
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::shape::RoundedRectangle;
use waterui_icon::SystemIcon;

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

    const fn icon_name(self) -> &'static str {
        match self {
            Self::Today => "calendar.circle.fill",
            Self::Scheduled => "calendar.badge.clock",
            Self::All => "tray.fill",
            Self::Flagged => "flag.fill",
            Self::Completed => "checkmark.circle.fill",
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

#[derive(Clone)]
struct SidebarRow {
    id: u32,
    dest: SidebarDestination,
    count: i32,
}

impl Identifiable for SidebarRow {
    type Id = u32;

    fn id(&self) -> Self::Id {
        self.id
    }
}

fn sidebar_rows() -> Vec<SidebarRow> {
    vec![
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

#[derive(Clone)]
struct ReminderRow {
    id: u64,
    title: &'static str,
    subtitle: Option<&'static str>,
    flagged: bool,
}

impl Identifiable for ReminderRow {
    type Id = u64;

    fn id(&self) -> Self::Id {
        self.id
    }
}

fn reminders_for(dest: SidebarDestination) -> (Vec<ReminderRow>, Vec<ReminderRow>) {
    match dest {
        SidebarDestination::Today => (
            vec![
                ReminderRow {
                    id: 1,
                    title: "Call dentist",
                    subtitle: Some("2:00 PM"),
                    flagged: false,
                },
                ReminderRow {
                    id: 2,
                    title: "Review water preview CLI crash path",
                    subtitle: Some("Before lunch"),
                    flagged: true,
                },
            ],
            vec![ReminderRow {
                id: 3,
                title: "Pick up package",
                subtitle: Some("Tomorrow 10:00 AM"),
                flagged: false,
            }],
        ),
        SidebarDestination::Scheduled => (
            vec![ReminderRow {
                id: 4,
                title: "Book flight",
                subtitle: Some("Fri"),
                flagged: false,
            }],
            vec![ReminderRow {
                id: 5,
                title: "Pay utilities",
                subtitle: Some("Next week"),
                flagged: false,
            }],
        ),
        SidebarDestination::All => (
            vec![
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
            vec![ReminderRow {
                id: 8,
                title: "Refactor toolbar metadata path",
                subtitle: Some("Cross-platform"),
                flagged: false,
            }],
        ),
        SidebarDestination::Flagged => (
            vec![ReminderRow {
                id: 9,
                title: "Prepare demo",
                subtitle: Some("High priority"),
                flagged: true,
            }],
            vec![],
        ),
        SidebarDestination::Completed => (
            vec![
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
            vec![],
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

fn filter_reminders(rows: Vec<ReminderRow>, normalized_query: Option<&str>) -> Vec<ReminderRow> {
    match normalized_query {
        Some(query) => rows
            .into_iter()
            .filter(|row| reminder_matches_query(row, query))
            .collect(),
        None => rows,
    }
}

fn main_view() -> impl View {
    let selection = binding(SidebarDestination::Today);
    let search = Binding::container(Str::default());

    hstack((
        sidebar(selection.clone(), search.clone()),
        Divider,
        content(selection, search),
    ))
}

fn sidebar(selection: Binding<SidebarDestination>, search: Binding<Str>) -> impl View {
    let rows = sidebar_rows();

    vstack((
        vstack((
            text("Reminders").title().bold().foreground(Foreground),
            TextField::new(&search).prompt("Search"),
            text("My Lists").caption().foreground(MutedForeground),
        ))
        .spacing(10.0)
        .padding_with(EdgeInsets::all(16.0)),
        List::for_each(rows, move |row| {
            let dest = row.dest;
            let is_selected = selection.clone().map(move |s| s == dest);
            let selection_for_action = selection.clone();

            ListItem::new(Dynamic::watch(is_selected, move |active| {
                let bg = if active {
                    Srgb::WHITE.with_opacity(0.14)
                } else {
                    Srgb::WHITE.with_opacity(0.0)
                };

                AnyView::new(
                    hstack((
                        SystemIcon::from_static(dest.icon_name())
                            .size(18.0, 18.0)
                            .foreground(dest.icon_color()),
                        text(dest.title()).body().foreground(Foreground),
                        spacer(),
                        text(format!("{}", row.count))
                            .caption()
                            .foreground(MutedForeground),
                    ))
                    .padding_with(EdgeInsets::symmetric(10.0, 14.0))
                    .background(bg)
                    .clip(RoundedRectangle::new(10.0))
                    .on_tap({
                        let selection_for_action = selection_for_action.clone();
                        move || selection_for_action.set(dest)
                    }),
                )
            }))
        }),
    ))
    .width(300.0)
    .background(Material::Thick)
}

fn content(selection: Binding<SidebarDestination>, search: Binding<Str>) -> impl View {
    Dynamic::watch(selection.zip(&search), move |(dest, query)| {
        let normalized_query = normalized_search_query(&query);
        let (today_rows, upcoming_rows) = reminders_for(dest);
        let today_rows = filter_reminders(today_rows, normalized_query.as_deref());
        let upcoming_rows = filter_reminders(upcoming_rows, normalized_query.as_deref());

        AnyView::new(
            vstack((
                content_header(dest),
                Divider,
                reminder_section("Today", today_rows),
                if upcoming_rows.is_empty() {
                    AnyView::new(spacer())
                } else {
                    AnyView::new(reminder_section("Upcoming", upcoming_rows))
                },
            ))
            .background(Material::Regular),
        )
    })
}

fn content_header(dest: SidebarDestination) -> impl View {
    vstack((
        hstack((
            text(dest.title()).title().bold().foreground(Foreground),
            spacer(),
            button(SystemIcon::PLUS.size(16.0, 16.0))
                .style(ButtonStyle::Borderless)
                .action(|| {}),
        )),
        text("Friday, February 6")
            .caption()
            .foreground(MutedForeground),
    ))
    .spacing(6.0)
    .padding_with(EdgeInsets::new(14.0, 18.0, 12.0, 18.0))
}

fn reminder_section(title: &'static str, rows: Vec<ReminderRow>) -> impl View {
    vstack((
        text(title)
            .caption()
            .bold()
            .foreground(MutedForeground)
            .padding_with(EdgeInsets::new(8.0, 18.0, 0.0, 18.0)),
        List::for_each(rows, |row| {
            ListItem::new(
                hstack((
                    SystemIcon::from_static("circle")
                        .size(16.0, 16.0)
                        .foreground(MutedForeground),
                    vstack((
                        text(row.title).body().foreground(Foreground),
                        row.subtitle
                            .map(|s| text(s).caption().foreground(MutedForeground)),
                    ))
                    .spacing(2.0),
                    spacer(),
                    if row.flagged {
                        AnyView::new(
                            SystemIcon::from_static("flag.fill")
                                .size(12.0, 12.0)
                                .foreground(Srgb::from_hex("#F28A34")),
                        )
                    } else {
                        AnyView::new(spacer().width(12.0))
                    },
                ))
                .padding_with(EdgeInsets::symmetric(10.0, 18.0)),
            )
        }),
    ))
}

pub fn app(env: Environment) -> App {
    App::new(main_view(), env)
}

waterui_ffi::export!();

#[preview]
fn reminders_preview() -> impl View {
    main_view()
}
