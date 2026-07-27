//! Large-data List example.
//!
//! This example exercises the behavior that distinguishes `List` from a
//! regular stack: lazy row realization, indexed jumps, deletion, and moving.

use waterui::Identifiable;
use waterui::app::App;
use waterui::component::list::{List, ListDelete, ListItem, ListMove};
use waterui::prelude::theme_color::{Foreground, MutedForeground};
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::reactive::collection::List as ReactiveList;

const DATASET_SIZE: usize = 100_000;

#[derive(Clone, Copy)]
struct Record {
    id: u64,
}

impl Identifiable for Record {
    type Id = u64;

    fn id(&self) -> Self::Id {
        self.id
    }
}

#[derive(Clone)]
struct DemoState {
    records: ReactiveList<Record>,
    remaining: Binding<i32>,
    editing: Binding<bool>,
    scroll: ScrollController<usize>,
}

impl DemoState {
    fn new() -> Self {
        let records = (0..DATASET_SIZE)
            .map(|index| Record { id: index as u64 })
            .collect::<Vec<_>>();
        Self {
            records: ReactiveList::from(records),
            remaining: binding(DATASET_SIZE as i32),
            editing: binding(false),
            scroll: ScrollController::new(0),
        }
    }
}

fn record_row(record: Record) -> ListItem {
    ListItem::new(
        vstack((
            text(format!("Record #{:06}", record.id))
                .sub_headline()
                .foreground(Foreground),
            text("Materialized only while this row is visible")
                .caption()
                .foreground(MutedForeground),
        ))
        .alignment(HorizontalAlignment::Leading)
        .padding_with(EdgeInsets::symmetric(10.0, 16.0)),
    )
}

fn delete_record(ListDelete(index): ListDelete, State(state): State<DemoState>) {
    let _ = state.records.remove(index);
    state.remaining.set(state.remaining.get() - 1);
}

fn move_record(ListMove(movement): ListMove, State(state): State<DemoState>) {
    let mut records = state.records.snapshot();
    let record = records.remove(movement.from());
    records.insert(movement.to(), record);
    let _ = state.records.replace(records);
}

fn jump_top(State(state): State<DemoState>) {
    state.scroll.scroll_to(0);
}

fn jump_middle(State(state): State<DemoState>) {
    state.scroll.scroll_to((state.remaining.get() as usize) / 2);
}

fn jump_last(State(state): State<DemoState>) {
    let remaining = state.remaining.get();
    if remaining > 0 {
        state.scroll.scroll_to(remaining as usize - 1);
    }
}

fn toggle_editing(State(state): State<DemoState>) {
    state.editing.set(!state.editing.get());
}

fn content(state: DemoState) -> impl View {
    let edit_label = state
        .editing
        .map(|editing| if editing { "Done" } else { "Edit" });
    let list = List::for_each(state.records.clone(), record_row)
        .editing(state.editing.clone())
        .on_delete(delete_record)
        .on_move(move_record)
        .scroll_controller(&state.scroll);

    vstack((
        vstack((
            text("100,000-row lazy List").title(),
            text!(
                "{remaining} active rows",
                remaining = state.remaining.clone()
            )
            .sub_headline()
            .foreground(MutedForeground),
            hstack((
                button("Top").bordered().action(jump_top),
                button("Middle").bordered().action(jump_middle),
                button("Last").bordered().action(jump_last),
                button(text!("{edit_label}"))
                    .bordered_prominent()
                    .action(toggle_editing),
            ))
            .spacing(8.0),
            text(
                "Animated jumps and the draggable scrollbar keep only viewport rows materialized.",
            )
            .caption()
            .foreground(MutedForeground),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(8.0)
        .padding(),
        Divider,
        list,
    ))
    .state(&state)
}

#[preview]
pub fn demo() -> impl View {
    content(DemoState::new())
}

pub fn app(env: Environment) -> App {
    let state = DemoState::new();
    App::new(move || content(state.clone()), env)
}
