//! The dispatch workloads both bench binaries measure: `dispatch` for
//! wall-clock time and allocations under divan, `dispatch_ir` for
//! instruction counts under gungraun.
//!
//! Every workload drives Hydrolysis's public runtimes, so the same code
//! measures any dispatch implementation behind them. The view a
//! measurement mounts is built by the caller outside the measured region:
//! what is measured is turning it into retained backend nodes, the
//! replacement of what it displaces, and the pump that does both.

use std::time::{Duration, Instant};

use hydrolysis::testing::TestStyle;
use hydrolysis::{FontFamilyResolution, HeadlessRuntime, SemanticRuntime};
use waterui::form::picker::picker;
use waterui::prelude::*;
use waterui::shape::{CornerRadius, RoundedRectangle, ShapeExt as _};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::GridRow;
use waterui_layout::scroll::ScrollController;

/// The window every workload mounts in.
const WIDTH: u32 = 400;
const HEIGHT: u32 = 800;

/// How far each pump advances the clock: past every default animation, so
/// a pump always settles the transition the previous one started and every
/// measurement sees the same retained tree.
const STEP: Duration = Duration::from_secs(1);

/// A semantic runtime whose root is one [`Dynamic`]: every
/// [`Remount::mount`] replaces the whole subtree and pumps once, the path a
/// `Dynamic` swap, a navigation destination or any other rebuild takes.
pub struct Remount {
    runtime: SemanticRuntime,
    handler: DynamicHandler<AnyView>,
    at: Instant,
}

impl Remount {
    #[must_use]
    pub fn new() -> Self {
        let (handler, dynamic) = Dynamic::new();
        let content = AnyViewBuilder::new(move || AnyView::new(dynamic.clone()));
        let runtime = SemanticRuntime::new(
            Environment::new(),
            content,
            WIDTH,
            HEIGHT,
            FontFamilyResolution::Strict,
        );
        let mut remount = Self {
            runtime,
            handler,
            at: Instant::now(),
        };
        remount.pump();
        remount
    }

    /// A runtime that has already mounted and dropped one `view` of the
    /// same shape, so a measurement sees warm caches rather than first-use
    /// costs.
    #[must_use]
    pub fn warmed<V: View>(view: V) -> Self {
        let mut remount = Self::new();
        remount.mount(view);
        remount.mount(());
        remount
    }

    /// Replaces the mounted subtree with `view` and pumps.
    pub fn mount(&mut self, view: impl View) {
        self.handler.set(AnyView::new(view));
        self.pump();
    }

    fn pump(&mut self) {
        self.at += STEP;
        let _ = self.runtime.pump_at(self.at);
    }
}

/// One text under twenty modifiers: the depth a styled leaf reaches.
pub fn modifier_chain() -> impl View {
    text("Modified")
        .padding()
        .foreground(Color::srgb(20, 40, 60))
        .background(Color::srgb(240, 240, 240))
        .opacity(0.9)
        .border(Color::srgb(10, 10, 10), 1.0)
        .padding()
        .scale(1.0)
        .rotation(Angle::ZERO)
        .offset(0.0, 0.0)
        .shadow(Shadow::default())
        .padding()
        .background(Color::srgb(250, 250, 250))
        .opacity(0.95)
        .border(Color::srgb(30, 30, 30), 1.0)
        .scale(1.0)
        .padding()
        .rotation(Angle::ZERO)
        .offset(0.0, 0.0)
        .disabled(false)
        .width(320.0)
}

/// A stack of one hundred children.
pub fn wide_stack() -> impl View {
    vstack(
        (0..100)
            .map(|index| text(format!("Row {index}")))
            .collect::<Vec<_>>(),
    )
}

/// The small subtree a `Dynamic` swaps in on every change.
pub fn replacement() -> impl View {
    hstack((text("Status"), spacer(), text("Ready")))
}

/// A settings screen: grouped toggles, sliders, steppers, fields and
/// pickers under section headers, in a scroll view.
pub fn settings_form() -> impl View {
    let groups = (0..6)
        .map(|group| {
            let enabled = Binding::bool(group % 2 == 0);
            let level = Binding::f64(0.5);
            let count = Binding::i32(group);
            let name = Binding::container(Str::from("Device"));
            let choice = Binding::i32(0);
            vstack((
                text(format!("Section {group}")),
                toggle("Enabled", &enabled),
                toggle("Notifications", &Binding::bool(true)),
                slider("Level", &level),
                stepper("Count", &count),
                field("Name", &name),
                picker(
                    "Mode",
                    vec![
                        text("Automatic").tag(0),
                        text("Manual").tag(1),
                        text("Off").tag(2),
                    ],
                    &choice,
                ),
                button("Reset").action(|| {}),
            ))
            .padding()
        })
        .collect::<Vec<_>>();
    scroll(vstack(groups))
}

/// A gallery page: a grid of cards, each a swatch, a title, a caption and
/// an action.
pub fn gallery_page() -> impl View {
    let rows = (0..8)
        .map(|row_index| {
            GridRow::new(
                (0..3)
                    .map(|column| {
                        let index = row_index * 3 + column;
                        vstack((
                            RoundedRectangle::new(CornerRadius::fraction(0.1))
                                .fill(Color::srgb(60, 120, 200))
                                .size(100.0, 80.0),
                            text(format!("Item {index}")),
                            text("Caption under the swatch").foreground(Color::srgb(90, 90, 90)),
                            button("Open").action(|| {}),
                        ))
                        .padding()
                        .background(Color::srgb(245, 245, 245))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    scroll(grid(3, rows))
}

/// A semantic runtime holding a navigation stack whose root page stays
/// mounted while a detail page is pushed and popped.
pub struct Navigation {
    runtime: SemanticRuntime,
    path: NavigationPath<u32>,
    at: Instant,
}

impl Navigation {
    #[must_use]
    pub fn new() -> Self {
        let path = NavigationPath::<u32>::new();
        let content = {
            let path = path.clone();
            AnyViewBuilder::new(move || {
                AnyView::new(
                    NavigationStack::with_path(
                        path.clone(),
                        NavigationView::new("Root", settings_form()),
                    )
                    .destination(|page: u32| {
                        NavigationView::new(format!("Page {page}"), gallery_page())
                    }),
                )
            })
        };
        let runtime = SemanticRuntime::new(
            Environment::new(),
            content,
            WIDTH,
            HEIGHT,
            FontFamilyResolution::Strict,
        );
        let mut navigation = Self {
            runtime,
            path,
            at: Instant::now(),
        };
        navigation.pump();
        navigation.push_and_pop();
        navigation
    }

    /// Pushes a detail page, settles, pops it and settles.
    pub fn push_and_pop(&mut self) {
        self.path.push(1);
        self.pump();
        let _ = self.path.pop();
        self.pump();
    }

    fn pump(&mut self) {
        self.at += STEP;
        let _ = self.runtime.pump_at(self.at);
    }
}

/// A semantic runtime showing a text bound to a counter: the steady-state
/// update dispatch must not touch, since it rebuilds nothing.
pub struct SignalUpdate {
    runtime: SemanticRuntime,
    count: Binding<i32>,
    value: i32,
    at: Instant,
}

impl SignalUpdate {
    #[must_use]
    pub fn new() -> Self {
        let count = Binding::i32(0);
        let content = {
            let count = count.clone();
            AnyViewBuilder::new(move || {
                let count = count.clone();
                AnyView::new(vstack((text!("Count {count}"), wide_stack())))
            })
        };
        let runtime = SemanticRuntime::new(
            Environment::new(),
            content,
            WIDTH,
            HEIGHT,
            FontFamilyResolution::Strict,
        );
        let mut update = Self {
            runtime,
            count,
            value: 0,
            at: Instant::now(),
        };
        update.pump();
        update.increment();
        update
    }

    /// Changes the counter and pumps the patch.
    pub fn increment(&mut self) {
        self.value += 1;
        self.count.set(self.value);
        self.pump();
    }

    fn pump(&mut self) {
        self.at += STEP;
        let _ = self.runtime.pump_at(self.at);
    }
}

/// Rows in the lazy list: far more than any fling reaches.
const LIST_ROWS: usize = 100_000;
/// Rows each scroll advances: about a viewport, so every step realizes a
/// fresh window of rows.
const LIST_STEP: usize = 20;

/// A rendered runtime holding a lazy list scrolled one viewport per step,
/// so each step realizes the rows a fling brings on screen.
///
/// Realizing rows needs layout, which only the rendered runtime has, so a
/// step also records and submits a frame through the GPU painter; its
/// counts include that submission on the measuring thread.
pub struct LazyRows {
    runtime: HeadlessRuntime,
    controller: ScrollController<usize>,
    row: usize,
    at: Instant,
}

impl LazyRows {
    #[must_use]
    pub fn new() -> Self {
        let controller = ScrollController::new(0usize);
        let content = {
            let controller = controller.clone();
            AnyViewBuilder::new(move || {
                let rows = (0..LIST_ROWS).map(SelfId::new).collect::<Vec<_>>();
                AnyView::new(
                    List::for_each(rows, |row: SelfId<usize>| {
                        let index = row.into_inner();
                        hstack((
                            text(format!("Row {index}")),
                            spacer(),
                            text("Detail").foreground(Color::srgb(90, 90, 90)),
                        ))
                    })
                    .scroll_controller(&controller),
                )
            })
        };
        let mut env = Environment::new();
        hydrolysis::testing::install_theme(&mut env);
        let runtime = HeadlessRuntime::new_for_tests(env, content, WIDTH, HEIGHT, TestStyle::new());
        let mut rows = Self {
            runtime,
            controller,
            row: 0,
            at: Instant::now(),
        };
        rows.pump();
        rows.advance();
        rows
    }

    /// Scrolls one viewport further and pumps, realizing the new rows.
    pub fn advance(&mut self) {
        self.row = (self.row + LIST_STEP) % LIST_ROWS;
        self.controller.scroll_to(self.row);
        self.pump();
    }

    fn pump(&mut self) {
        self.at += STEP;
        let _ = self.runtime.pump_at(false, self.at);
    }
}
