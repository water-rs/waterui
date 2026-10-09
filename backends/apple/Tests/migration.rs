//! Native behavior migrated from the former Swift backend tests.

use crate::{HostView, mtm};
use libtest_mimic::Trial;

pub fn trials() -> Vec<Trial> {
    let tests = vec![
        Trial::test("migration::signals::silent_watch", || {
            signals::watch_sees_updates_but_not_the_present_value();
            Ok(())
        }),
        Trial::test("migration::signals::ordered_updates", || {
            signals::a_watcher_receives_every_update_in_order();
            Ok(())
        }),
        Trial::test("migration::signals::drop_cancels", || {
            signals::dropping_the_leaf_cancels_its_watchers();
            Ok(())
        }),
        Trial::test("migration::signals::independent_owners", || {
            signals::independent_owners();
            Ok(())
        }),
        Trial::test("migration::signals::owned_values", || {
            signals::owned_values();
            Ok(())
        }),
        Trial::test("migration::color::native_hdr_and_updates", || {
            colors::native_hdr_and_updates();
            Ok(())
        }),
        Trial::test("migration::color::srgb_known_values", || {
            colors::srgb_known_values();
            Ok(())
        }),
    ];
    #[cfg(target_os = "macos")]
    let tests = {
        let mut tests = tests;
        tests.push(Trial::test(
            "migration::color::native_well_round_trip",
            || {
                colors::native_well_round_trip();
                Ok(())
            },
        ));
        tests
    };
    #[cfg(target_os = "ios")]
    let tests = {
        let mut tests = tests;
        tests.extend([
            Trial::test("migration::uikit::nested_list_frames", || {
                uikit_surface::list_cells_give_nested_text_real_frames();
                Ok(())
            }),
            Trial::test("migration::uikit::plain_field_matches_swiftui", || {
                uikit_surface::text_field_renders_plain_with_a_real_height();
                Ok(())
            }),
            Trial::test("migration::uikit::list_chrome_matches_swiftui", || {
                uikit_surface::list_row_height_pitches_and_respects_the_floor();
                Ok(())
            }),
            Trial::test("migration::uikit::compact_split", || {
                uikit_surface::compact_split_follows_selection();
                Ok(())
            }),
            Trial::test("migration::uikit::stable_id_row_replace", || {
                uikit_surface::stable_id_payload_replace_remateries_the_row();
                Ok(())
            }),
            Trial::test("migration::uikit::stable_id_row_replace_sectioned", || {
                uikit_surface::stable_id_payload_replace_in_sectioned_rows();
                Ok(())
            }),
            Trial::test("migration::uikit::stable_id_row_replace_reentrant", || {
                uikit_surface::stable_id_payload_replace_under_reentrant_emission();
                Ok(())
            }),
        ]);
        tests.extend(uikit_reentrant_trials());
        tests
    };
    tests
}

/// The `#319` deferred-delivery trials — every consumer's emission must
/// record and apply at the next safe boundary.
#[cfg(target_os = "ios")]
fn uikit_reentrant_trials() -> [Trial; 4] {
    [
        Trial::test(
            "migration::uikit::reentrant_remove_retained_snapshot",
            || {
                uikit_surface::reentrant_remove_realizes_the_retained_snapshot();
                Ok(())
            },
        ),
        Trial::test(
            "migration::uikit::reentrant_container_apply_at_boundary",
            || {
                uikit_surface::reentrant_container_remove_applies_at_boundary();
                Ok(())
            },
        ),
        Trial::test(
            "migration::uikit::reentrant_table_rows_apply_at_boundary",
            || {
                uikit_surface::reentrant_table_rows_apply_at_boundary();
                Ok(())
            },
        ),
        Trial::test(
            "migration::uikit::reentrant_columns_and_label_apply_at_boundary",
            || {
                uikit_surface::reentrant_columns_and_label_apply_at_boundary();
                Ok(())
            },
        ),
    ]
}

/// Signal subscription semantics at the typed contract — the native
/// equivalents of the removed C-wire `WuiSignal`/`WuiCancellation` cases:
/// `leaf.watch` registers without delivering the current value, updates
/// arrive synchronously in order, and dropping the leaf (the guard's
/// owner) is the cancellation.
mod signals {
    use std::cell::RefCell;
    use std::rc::Rc;

    use waterui::reactive::binding;
    use waterui_apple::contract::NativeLeaf;

    use super::{HostView, mtm};
    use crate::leaf::TestSubView;

    #[derive(Debug)]
    struct Released {
        id: i32,
        drops: Rc<RefCell<Vec<i32>>>,
    }

    impl Drop for Released {
        fn drop(&mut self) {
            self.drops.borrow_mut().push(self.id);
        }
    }

    pub fn independent_owners() {
        let host = HostView::new(mtm(), cocoa_ui::Rect::ZERO);
        let source = binding(5);
        let drops = Rc::new(RefCell::new(Vec::new()));
        let first_seen = Rc::new(RefCell::new(Vec::new()));
        let second_seen = Rc::new(RefCell::new(Vec::new()));
        let mut first = NativeLeaf::new(&*host, TestSubView);
        let mut second = NativeLeaf::new(&*host, TestSubView);
        for (id, leaf, seen) in [
            (1, &mut first, first_seen.clone()),
            (2, &mut second, second_seen.clone()),
        ] {
            let released = Released {
                id,
                drops: drops.clone(),
            };
            leaf.bind(&source, move |value| {
                // Capture the whole resource so the watcher owns its destructor.
                let _owned = &released;
                seen.borrow_mut().push(value);
            });
        }
        assert_eq!(*first_seen.borrow(), [5]);
        assert_eq!(*second_seen.borrow(), [5]);
        source.set(9);
        assert_eq!(*first_seen.borrow(), [5, 9]);
        assert_eq!(*second_seen.borrow(), [5, 9]);
        assert!(drops.borrow().is_empty());
        let mut first = Some(first);
        drop(first.take());
        drop(first.take());
        assert_eq!(*drops.borrow(), [1], "one owner releases exactly once");
        source.set(12);
        assert_eq!(
            *first_seen.borrow(),
            [5, 9],
            "removed watcher is never called"
        );
        assert_eq!(
            *second_seen.borrow(),
            [5, 9, 12],
            "other owner remains active"
        );
        drop(second);
        assert_eq!(*drops.borrow(), [1, 2]);
    }

    pub fn owned_values() {
        let host = HostView::new(mtm(), cocoa_ui::Rect::ZERO);
        let mut leaf = NativeLeaf::new(&*host, TestSubView);
        let drops = Rc::new(RefCell::new(Vec::new()));
        let value = |id| {
            Rc::new(Released {
                id,
                drops: drops.clone(),
            })
        };
        let source: waterui::reactive::Binding<Rc<Released>> = binding(value(1));
        let displayed = Rc::new(RefCell::new(None));
        let target = displayed.clone();
        leaf.bind(&source, move |value| *target.borrow_mut() = Some(value));
        assert_eq!(displayed.borrow().as_ref().unwrap().id, 1);
        source.set(value(2));
        assert_eq!(
            *drops.borrow(),
            [1],
            "replacing releases the previous value"
        );
        source.set(value(3));
        assert_eq!(*drops.borrow(), [1, 2]);
        assert_eq!(displayed.borrow().as_ref().unwrap().id, 3);
        let weak = Rc::downgrade(&displayed);
        drop(displayed);
        assert!(weak.upgrade().is_some(), "binding owns its native target");
        drop(leaf);
        assert!(
            weak.upgrade().is_none(),
            "leaf destruction releases its target"
        );
        drop(source);
        assert_eq!(
            *drops.borrow(),
            [1, 2, 3],
            "final value releases exactly once"
        );
    }

    /// `watch` subscribes without replaying the present value — the first
    /// call is the first `set`. (Initial delivery is `bind`'s contract,
    /// asserted by `leaf::bind_applies_now_and_on_every_change`.)
    pub fn watch_sees_updates_but_not_the_present_value() {
        let mtm = mtm();
        let host = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let mut leaf = NativeLeaf::new(&*host, TestSubView);
        let flag = binding(false);
        let seen = Rc::new(RefCell::new(Vec::new()));
        {
            let seen = Rc::clone(&seen);
            leaf.watch(&flag, move |ctx| seen.borrow_mut().push(ctx.into_value()));
        }
        assert!(
            seen.borrow().is_empty(),
            "watch fired before any change was written"
        );
        flag.set(true);
        assert_eq!(*seen.borrow(), vec![true]);
    }

    /// Updates arrive synchronously inside `set`, in write order, to every
    /// watcher — several updates in a row all land.
    pub fn a_watcher_receives_every_update_in_order() {
        let mtm = mtm();
        let host = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let mut leaf = NativeLeaf::new(&*host, TestSubView);
        let counter = binding(0_i32);
        let seen = Rc::new(RefCell::new(Vec::new()));
        {
            let seen = Rc::clone(&seen);
            leaf.watch(&counter, move |ctx| {
                seen.borrow_mut().push(ctx.into_value());
            });
        }
        for value in [1, 2, 3] {
            counter.set(value);
            // synchronous: the write's watcher has already run
            assert_eq!(seen.borrow().last(), Some(&value));
        }
        assert_eq!(*seen.borrow(), vec![1, 2, 3]);
    }

    /// Dropping the leaf drops the watcher guards it keeps — the cancel /
    /// deinit contract: a write after the leaf is gone reaches nobody.
    pub fn dropping_the_leaf_cancels_its_watchers() {
        let mtm = mtm();
        let host = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let flag = binding(false);
        let seen = Rc::new(RefCell::new(Vec::new()));
        {
            let mut leaf = NativeLeaf::new(&*host, TestSubView);
            {
                let seen = Rc::clone(&seen);
                leaf.watch(&flag, move |ctx| {
                    seen.borrow_mut().push(ctx.into_value());
                });
            }
            flag.set(true);
            drop(leaf);
        }
        flag.set(false);
        assert_eq!(
            *seen.borrow(),
            vec![true],
            "a dropped leaf's watchers must not be reached"
        );
    }
}

/// `UIKit` render assertions — ports of the hosted `WaterUITests` cases
/// that introspected a running app's view tree. The typed contract makes
/// the same tree reachable in-process: `dispatch::render` hands back the
/// leaf, and mounting it into a real `UIWindow` gives it the trait
/// collections and layout the hosted app gave it.
#[cfg(target_os = "ios")]
mod uikit_surface {
    use cocoa_ui::objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use cocoa_ui::objc2_foundation::NSIndexPath;
    use cocoa_ui::objc2_ui_kit::{
        NSIndexPathUIKitAdditions, UILabel, UITableView, UITextBorderStyle,
    };
    use waterui::Str;
    use waterui::component::list::{List, ListItem};
    use waterui::prelude::theme_color::{Accent, Foreground};
    use waterui::prelude::*;
    use waterui::reactive::binding;
    use waterui::shape::Circle;
    use waterui_apple::native_test_support::{
        MAIN_QUEUE_DEADLINE, drain_main_queue, pump_main_until,
    };
    use waterui_backend_core::AnyView;

    use crate::{PlatformView, Retained, mtm};
    use std::cell::Cell;
    use std::rc::Rc;
    use waterui::Identifiable;
    use waterui::reactive::collection::{Collection, List as ReactiveList};

    /// `id`-keyed row whose payload flips between a short label and a tall
    /// one — the stable-id replace regression probe.
    #[derive(Clone, Copy, Identifiable)]
    struct ProbeRow {
        #[id]
        id: u64,
        tall: bool,
    }

    /// A row whose body mutates the source collection once, during
    /// materialization — the reentrant-emission probe for the pending
    /// delivery queue.
    struct ReentrantRow {
        items: ReactiveList<ProbeRow>,
        fired: Rc<Cell<bool>>,
    }

    impl waterui::View for ReentrantRow {
        fn body(self, _env: &waterui::Environment) -> impl waterui::View {
            if !self.fired.replace(true) {
                let _ = self.items.set(0, ProbeRow { id: 20, tall: true });
            }
            text("reentrant row")
        }
    }

    /// A row whose body removes the first item once, during
    /// materialization — the reentrant-membership probe: rows realized
    /// while its emission is still queued must read the retained
    /// snapshot, never the live source.
    struct RemovingRow {
        items: ReactiveList<ProbeRow>,
        fired: Rc<Cell<bool>>,
    }

    impl waterui::View for RemovingRow {
        fn body(self, _env: &waterui::Environment) -> impl waterui::View {
            if !self.fired.replace(true) {
                let _ = self.items.remove(0);
            }
            text("removing row")
        }
    }

    /// Every descendant of `view`, depth-first — the hosted suite's tree
    /// walk, here over the leaf's own view.
    fn descendants(view: &PlatformView) -> Vec<Retained<PlatformView>> {
        let mut all = Vec::new();
        let mut stack = vec![cocoa_ui::view::retain_base(view)];
        while let Some(next) = stack.pop() {
            stack.extend(cocoa_ui::view::subviews(&next));
            all.push(next);
        }
        all
    }

    fn table(root: &PlatformView) -> Retained<UITableView> {
        descendants(root)
            .into_iter()
            .find_map(|view| view.downcast::<UITableView>().ok())
            .expect("the list leaf mounts a UITableView")
    }

    fn reference(key: &str) -> f64 {
        let path = std::env::var("WATERUI_REFERENCE_METRICS")
            .expect("run prepare-native-reference.sh on this simulator first");
        let metrics: serde_json::Value = serde_json::from_slice(
            &std::fs::read(path).expect("read live SwiftUI reference metrics"),
        )
        .expect("valid reference metrics");
        metrics[key].as_f64().expect("reference metric exists")
    }

    fn close(actual: f64, metric: &str) {
        let expected = reference(metric);
        assert!(
            (actual - expected).abs() <= 0.5,
            "{metric}: actual {actual} must match live platform reference {expected} within 0.5pt"
        );
    }

    /// The device fixture's row: `Label` wrapping
    /// `hstack(icon, vstack(hstack(text, spacer, flag), text, text))` —
    /// the nested-stack shape whose inner text lost its frames.
    fn message_row(sender: &'static str, subject: &'static str, preview: &'static str) -> ListItem {
        ListItem::new(Label::new(
            Str::from(format!("{sender}: {subject}")),
            move || {
                hstack((
                    hstack((Accent.size(8.0, 8.0).clip(Circle),)).size(8.0, 8.0),
                    vstack((
                        hstack((
                            text(sender).sub_headline().foreground(Foreground),
                            spacer(),
                            text("flag").caption().muted(),
                        ))
                        .spacing(6.0),
                        text(subject).body().foreground(Foreground),
                        text(preview).caption().muted(),
                    ))
                    .leading()
                    .spacing(2.0),
                ))
                .top()
                .spacing(6.0)
                .padding_vertical(8.0)
            },
        ))
    }

    /// The hosted list: five of the nested-stack rows, the same content
    /// `IOSTestHost` packages for the device lane.
    fn inbox() -> impl View {
        List::content((
            || {
                message_row(
                    "Ada Lovelace",
                    "WaterUI render loop",
                    "The nested vstack inside this row must paint its text.",
                )
            },
            || {
                message_row(
                    "Grace Hopper",
                    "List cell layout",
                    "Three lines sit in a vstack nested in the row's hstack.",
                )
            },
            || {
                message_row(
                    "Edsger Dijkstra",
                    "Placement proposals",
                    "Every nested stack receives the width its parent proposes.",
                )
            },
            || {
                message_row(
                    "Barbara Liskov",
                    "Substitution",
                    "Cells measure correctly; their text must render too.",
                )
            },
            || {
                message_row(
                    "Margaret Hamilton",
                    "Priority display",
                    "Zero-width frames are the regression this fixture guards.",
                )
            },
        ))
    }

    /// Every nonempty label in every visible cell retains a nonempty,
    /// intersecting frame, including the nested inbox stacks.
    pub fn list_cells_give_nested_text_real_frames() {
        let env = crate::resolve::env();
        let mount = waterui_apple::native_test_support::mount_uikit_in(
            mtm(),
            AnyView::new(inbox()),
            &env,
            cocoa_ui::Rect::new(0.0, 0.0, 393.0, 852.0),
        );
        let table = table(mount.content.view());
        let cells = table.visibleCells();
        assert!(cells.count() > 0, "the list materializes visible cells");
        let mut labels = 0;
        for cell in cells {
            for view in descendants(&cell.contentView()) {
                let Some(label) = view.downcast_ref::<UILabel>() else {
                    continue;
                };
                if label.text().is_none_or(|text| text.is_empty()) {
                    continue;
                }
                labels += 1;
                assert!(
                    view.frame().size.width > 0.0 && view.frame().size.height > 0.0,
                    "a nonempty label has an empty frame"
                );
                let frame = cocoa_ui::view::convert_rect(&view, view.bounds().into(), Some(&cell));
                let bounds = cell.bounds();
                assert!(
                    frame.origin.x < bounds.origin.x + bounds.size.width + 1.0
                        && frame.origin.x + frame.size.width > bounds.origin.x - 1.0
                        && frame.origin.y < bounds.origin.y + bounds.size.height + 1.0
                        && frame.origin.y + frame.size.height > bounds.origin.y - 1.0,
                    "a nonempty label does not intersect its cell"
                );
            }
        }
        assert!(
            labels >= 3,
            "expected at least three nonempty labels, found {labels}"
        );
    }

    /// Compare the actual plain field to the independently hosted `SwiftUI` field.
    pub fn text_field_renders_plain_with_a_real_height() {
        let value = binding(Str::from("x"));
        let env = crate::resolve::env();
        let mount = waterui_apple::native_test_support::mount_uikit_in(
            mtm(),
            AnyView::new(TextField::new("", &value)),
            &env,
            cocoa_ui::Rect::new(0.0, 0.0, 402.0, 874.0),
        );
        let field = descendants(mount.content.view())
            .into_iter()
            .find_map(|view| view.downcast::<cocoa_ui::uikit::TextField>().ok())
            .expect("the text field leaf mounts the kit UITextField");
        assert_eq!(field.borderStyle(), UITextBorderStyle::None);
        assert!(field.layer().borderWidth().abs() <= f64::EPSILON);
        if let Some(background) = field.backgroundColor() {
            // SAFETY: the retained UIKit color is read on the actual main thread.
            let color = unsafe { background.CGColor() };
            assert!(objc2_core_graphics::CGColor::alpha(Some(&color)) <= f64::EPSILON);
        }
        let bounds = CGRect::new(CGPoint::ZERO, CGSize::new(402.0, 60.0));
        for rect in [
            field.textRectForBounds(bounds),
            field.editingRectForBounds(bounds),
        ] {
            assert!((rect.origin.x - bounds.origin.x).abs() <= f64::EPSILON);
            assert!((rect.origin.y - bounds.origin.y).abs() <= f64::EPSILON);
            assert!((rect.size.width - bounds.size.width).abs() <= f64::EPSILON);
            assert!((rect.size.height - bounds.size.height).abs() <= f64::EPSILON);
        }
        close(
            field.sizeThatFits(CGSize::new(402.0, f64::MAX)).height,
            "textFieldHeight",
        );
    }

    /// Platform margins, 24pt row pitch, and the 4pt-content minimum all
    /// compare against actual displayed UIKit/SwiftUI reference rows.
    pub fn list_row_height_pitches_and_respects_the_floor() {
        for (height, metric) in [(24.0_f32, "row24Height"), (4.0, "row4Height")] {
            let env = crate::resolve::env();
            let mount = waterui_apple::native_test_support::mount_uikit_in(
                mtm(),
                AnyView::new(List::content((move || {
                    ListItem::new(Color::srgb(255, 0, 0).height(height))
                },))),
                &env,
                cocoa_ui::Rect::new(0.0, 0.0, 402.0, 874.0),
            );
            let table = table(mount.content.view());
            let cells = table.visibleCells();
            assert_eq!(cells.count(), 1, "one reference row materializes");
            let cell = cells.objectAtIndex(0);
            close(cell.frame().size.height, metric);
            let cell = cell
                .downcast_ref::<cocoa_ui::uikit::TableCell>()
                .expect("the backend uses the kit's row cell");
            let hosted = cell.content().expect("the cell owns rendered content");
            let content = cell.contentView();
            let rect =
                cocoa_ui::view::convert_rect(&hosted, hosted.bounds().into(), Some(&content));
            let bounds = content.bounds();
            close(rect.origin.y - bounds.origin.y, "rowTop");
            close(rect.origin.x - bounds.origin.x, "rowLeading");
            close(
                bounds.origin.y + bounds.size.height - rect.origin.y - rect.size.height,
                "rowBottom",
            );
            close(
                bounds.origin.x + bounds.size.width - rect.origin.x - rect.size.width,
                "rowTrailing",
            );
        }
    }

    pub fn compact_split_follows_selection() {
        use cocoa_ui::objc2_ui_kit::{
            UINavigationController, UISplitViewController, UISplitViewControllerColumn,
            UIViewController,
        };
        use waterui::navigation::{NavigationSplitView, NavigationView};

        fn column_root(
            split: &UISplitViewController,
            column: UISplitViewControllerColumn,
        ) -> Retained<UIViewController> {
            let controller = split
                .viewControllerForColumn(column)
                .expect("column exists");
            controller
                .downcast_ref::<UINavigationController>()
                .map_or_else(
                    || controller.clone(),
                    |nav| {
                        nav.viewControllers()
                            .firstObject()
                            .expect("column root exists")
                    },
                )
        }

        fn visible(controller: &UIViewController) -> bool {
            controller.viewIfLoaded().is_some_and(|view| {
                view.window().is_some() && !cocoa_ui::view::is_hidden_in_hierarchy(&view)
            })
        }

        let selection = binding(Some(1_i32));
        let split = NavigationSplitView::new(&selection, text("Sidebar"), |id: i32| {
            NavigationView::new(format!("Detail {id}"), text("detail"))
        });
        let env = crate::resolve::env();
        let mount = waterui_apple::native_test_support::mount_uikit_in(
            mtm(),
            AnyView::new(split),
            &env,
            cocoa_ui::Rect::new(0.0, 0.0, 393.0, 852.0),
        );
        let controller = descendants(mount.content.view())
            .into_iter()
            .find_map(|view| {
                cocoa_ui::uikit::view_controller::enclosing_controller(&view)
                    .and_then(|vc| vc.downcast::<UISplitViewController>().ok())
            })
            .expect("the split owns a UISplitViewController");
        assert!(
            controller.isCollapsed(),
            "a compact window collapses the split"
        );
        let sidebar = column_root(&controller, UISplitViewControllerColumn::Primary);
        let detail = column_root(&controller, UISplitViewControllerColumn::Secondary);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || visible(&detail)
                && !visible(&sidebar)
                && detail.transitionCoordinator().is_none()),
            "Some initially shows only the detail"
        );
        assert_eq!(selection.snapshot(), Some(1));

        selection.set(None);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || visible(&sidebar)
                && !visible(&detail)
                && sidebar.transitionCoordinator().is_none()),
            "setting None returns to only the sidebar"
        );

        selection.set(Some(2));
        assert!(
            drain_main_queue(mtm()),
            "the selection update drains the main queue"
        );
        let detail = column_root(&controller, UISplitViewControllerColumn::Secondary);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || visible(&detail)
                && !visible(&sidebar)
                && detail.transitionCoordinator().is_none()),
            "setting Some opens only the new detail"
        );
        assert_eq!(selection.snapshot(), Some(2));

        let nav = controller
            .childViewControllers()
            .firstObject()
            .expect("the compact split has a child controller")
            .downcast::<UINavigationController>()
            .expect("the compact split navigates between its columns");
        assert!(
            nav.popViewControllerAnimated(false).is_some(),
            "native back pops the detail column"
        );
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || selection.snapshot().is_none()
                && visible(&sidebar)
                && !visible(&detail)),
            "native back clears selection and returns to only the sidebar"
        );
    }

    /// `items.set` on a surviving `ItemId` is a `replaced` change: the row's
    /// mounted leaf must be re-materialized and its measured contract
    /// re-derived, not reused — the `#306` regression.
    pub fn stable_id_payload_replace_remateries_the_row() {
        let items = ReactiveList::from(vec![
            ProbeRow { id: 1, tall: false },
            ProbeRow { id: 2, tall: false },
        ]);
        let list = List::for_each(items.clone(), |row: ProbeRow| {
            let content: AnyView = if row.tall {
                AnyView::new(text("updated tall row").height(60.0))
            } else {
                AnyView::new(text("row"))
            };
            ListItem::new(content)
        });
        let env = crate::resolve::env();
        let mount = waterui_apple::native_test_support::mount_uikit_in(
            mtm(),
            AnyView::new(list),
            &env,
            cocoa_ui::Rect::new(0.0, 0.0, 393.0, 852.0),
        );
        let table = table(mount.content.view());
        let first_row = NSIndexPath::indexPathForRow_inSection(0, 0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || cell_height(&table, &first_row)
                .is_some()),
            "the first row materializes once deferred work drains"
        );
        let first = table
            .cellForRowAtIndexPath(&first_row)
            .expect("the first row stays mounted");
        let short = first.frame().size.height;

        let _ = items.set(0, ProbeRow { id: 1, tall: true });
        mount.window.layoutIfNeeded();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                cell_height(&table, &first_row).is_some_and(|height| height > short + 10.0)
                    && cell_shows(&table, &first_row, "updated tall row")
            }),
            "the replaced row re-materializes on its new contract"
        );

        let cell = table
            .cellForRowAtIndexPath(&first_row)
            .expect("the replaced row stays mounted");
        let tall = cell.frame().size.height;
        assert!(
            tall > short + 10.0,
            "the replaced row re-measures on its new contract ({short} -> {tall})"
        );
        assert!(
            label_texts(&cell.contentView())
                .iter()
                .any(|text| text.contains("updated tall row")),
            "the replaced row re-materializes its leaf"
        );

        // Coalesced emissions: remove+reinsert of a stable id in the same
        // turn leaves membership unchanged, so only the `inserted` position
        // marks the row dirty — the leaf must still be re-materialized.
        let _ = items.remove(0);
        items.insert(0, ProbeRow { id: 1, tall: false });
        mount.window.layoutIfNeeded();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                cell_height(&table, &first_row).is_some_and(|height| (height - short).abs() < 1.0)
                    && !cell_shows(&table, &first_row, "updated tall row")
            }),
            "the reinserted row re-materializes on its own contract"
        );
        let cell = table
            .cellForRowAtIndexPath(&first_row)
            .expect("the reinserted row stays mounted");
        let back = cell.frame().size.height;
        assert!(
            (back - short).abs() < 1.0,
            "the reinserted row returns to its own contract ({short} -> {back})"
        );
        assert!(
            !label_texts(&cell.contentView())
                .iter()
                .any(|text| text.contains("updated tall row")),
            "the reinserted row does not keep the replaced leaf"
        );

        mixed_membership_turn_lands_each_contract(&mount, &table, &items, &first_row, short);
    }

    /// Mixed replacement + membership + new id in one turn: the newest
    /// snapshot must win over every queued emission.
    fn mixed_membership_turn_lands_each_contract(
        mount: &waterui_apple::native_test_support::UIKitMount,
        table: &UITableView,
        items: &ReactiveList<ProbeRow>,
        first_row: &NSIndexPath,
        short: f64,
    ) {
        let _ = items.set(1, ProbeRow { id: 2, tall: true });
        let _ = items.remove(0);
        items.push(ProbeRow { id: 3, tall: false });
        mount.window.layoutIfNeeded();
        let pushed_row = NSIndexPath::indexPathForRow_inSection(1, 0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                cell_height(table, first_row).is_some_and(|height| height > short + 10.0)
                    && cell_height(table, &pushed_row)
                        .is_some_and(|height| (height - short).abs() < 1.0)
            }),
            "the moved and pushed rows land on their own contracts"
        );
        // The newest state has landed; an older queued flush would apply
        // after it. Drain the main queue past every queued emission and lay
        // out again so such an overwrite would show in the cells below.
        assert!(
            drain_main_queue(mtm()),
            "the queued emissions drain off the main queue"
        );
        mount.window.layoutIfNeeded();
        let moved = table
            .cellForRowAtIndexPath(first_row)
            .expect("the moved row stays mounted");
        let moved_height = moved.frame().size.height;
        assert!(
            moved_height > short + 10.0,
            "id 2 carries its replaced contract to index 0 ({moved_height})"
        );
        let fresh = table
            .cellForRowAtIndexPath(&pushed_row)
            .expect("the pushed row is mounted");
        assert!(
            (fresh.frame().size.height - short).abs() < 1.0,
            "the pushed row measures on its own contract"
        );
    }

    /// The same stable-id replace contract through the sectioned
    /// `reloadData` path — a multi-group list must still evict the replaced
    /// row's measured contract — a `#307` regression.
    pub fn stable_id_payload_replace_in_sectioned_rows() {
        let env = crate::resolve::env();

        // Sectioned shape: a section marker produces a multi-group list,
        // which applies changes through the whole-table `reloadData` path —
        // a stable-id replace must still evict the row's measured contract.
        let sectioned = ReactiveList::from(vec![
            ProbeRow { id: 10, tall: true },
            ProbeRow {
                id: 11,
                tall: false,
            },
        ]);
        let list = List::new(waterui::views::ForEach::new(
            sectioned.clone(),
            |row: ProbeRow| {
                let content: AnyView = if row.tall {
                    AnyView::new(text("updated tall row").height(60.0))
                } else {
                    AnyView::new(text("row"))
                };
                let item = ListItem::new(content);
                if row.id == 10 {
                    item.section(waterui::component::list::ListSection::new("Sectioned"))
                } else {
                    item
                }
            },
        ));
        let mount = waterui_apple::native_test_support::mount_uikit_in(
            mtm(),
            AnyView::new(list),
            &env,
            cocoa_ui::Rect::new(0.0, 0.0, 393.0, 852.0),
        );
        let sectioned_table = table(mount.content.view());
        let untouched_row = NSIndexPath::indexPathForRow_inSection(1, 0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || cell_height(
                &sectioned_table,
                &untouched_row
            )
            .is_some()),
            "the sectioned rows materialize once deferred work drains"
        );
        // The marker labels the group id 10 starts; id 11 has no marker and
        // stays inside it — the labeled group takes the `reloadData` path.
        let short = sectioned_table
            .cellForRowAtIndexPath(&untouched_row)
            .expect("the untouched sectioned row is mounted")
            .frame()
            .size
            .height;
        let _ = sectioned.set(
            0,
            ProbeRow {
                id: 10,
                tall: false,
            },
        );
        mount.window.layoutIfNeeded();
        let replaced_row = NSIndexPath::indexPathForRow_inSection(0, 0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                cell_height(&sectioned_table, &replaced_row)
                    .is_some_and(|height| (height - short).abs() < 1.0)
                    && sectioned_table
                        .cellForRowAtIndexPath(&replaced_row)
                        .is_some_and(|cell| {
                            label_texts(&cell.contentView())
                                .iter()
                                .any(|text| text == "row")
                        })
            }),
            "the replaced sectioned row re-materializes on its short contract"
        );
        let cell = sectioned_table
            .cellForRowAtIndexPath(&replaced_row)
            .expect("the replaced sectioned row stays mounted");
        let sectioned_height = cell.frame().size.height;
        let sectioned_text = label_texts(&cell.contentView());
        assert!(
            (sectioned_height - short).abs() < 1.0,
            "a replaced sectioned row re-measures under reloadData: {sectioned_height} vs {short}"
        );
        assert!(
            sectioned_text.iter().any(|t| t == "row"),
            "the sectioned replace shows the new payload, got {sectioned_text:?}"
        );
    }

    /// A row whose `View::body` mutates the source during materialization
    /// records a reentrant emission; a newer update recorded before that
    /// queued flush drains must win — a `#307` regression.
    pub fn stable_id_payload_replace_under_reentrant_emission() {
        let env = crate::resolve::env();
        let fired = Rc::new(Cell::new(false));
        let items = ReactiveList::from(vec![
            ProbeRow {
                id: 20,
                tall: false,
            },
            ProbeRow {
                id: 21,
                tall: false,
            },
        ]);
        let list = List::for_each(items.clone(), {
            let items = items.clone();
            let fired = Rc::clone(&fired);
            move |row: ProbeRow| {
                let content: AnyView = if row.tall {
                    AnyView::new(text("updated tall row").height(60.0))
                } else if row.id == 20 {
                    AnyView::new(ReentrantRow {
                        items: items.clone(),
                        fired: fired.clone(),
                    })
                } else {
                    AnyView::new(text("row"))
                };
                ListItem::new(content)
            }
        });
        let mount = waterui_apple::native_test_support::mount_uikit_in(
            mtm(),
            AnyView::new(list),
            &env,
            cocoa_ui::Rect::new(0.0, 0.0, 393.0, 852.0),
        );
        let reentrant_table = table(mount.content.view());
        let first_row = NSIndexPath::indexPathForRow_inSection(0, 0);
        let untouched_row = NSIndexPath::indexPathForRow_inSection(1, 0);
        // Pumping until the re-emitted payload lands materializes the rows
        // and drains the body's reentrant emission — the update below is
        // then unambiguously newer, and an older queued snapshot may never
        // overwrite it.
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                cell_shows(&reentrant_table, &first_row, "updated tall row")
                    && cell_height(&reentrant_table, &untouched_row).is_some()
            }),
            "the body's reentrant emission applies while the rows materialize"
        );
        // Row 1 (id 21) is untouched at this point — its contract is the
        // short baseline for this mounted list.
        let short = reentrant_table
            .cellForRowAtIndexPath(&untouched_row)
            .expect("the untouched row is mounted")
            .frame()
            .size
            .height;
        let _ = items.set(
            0,
            ProbeRow {
                id: 20,
                tall: false,
            },
        );
        let _ = items.set(1, ProbeRow { id: 21, tall: true });
        mount.window.layoutIfNeeded();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                cell_height(&reentrant_table, &first_row)
                    .is_some_and(|height| (height - short).abs() < 1.0)
                    && cell_shows(&reentrant_table, &first_row, "reentrant row")
                    && cell_height(&reentrant_table, &untouched_row)
                        .is_some_and(|height| height > short + 10.0)
            }),
            "the newest emission wins over the queued reentrant one"
        );
        // The newest state has landed; the queued reentrant flush would
        // apply after it. Drain the main queue past every queued emission
        // and lay out again so such an overwrite would show in the cells
        // below.
        assert!(
            drain_main_queue(mtm()),
            "the queued emissions drain off the main queue"
        );
        mount.window.layoutIfNeeded();
        let cell = reentrant_table
            .cellForRowAtIndexPath(&first_row)
            .expect("the reentrant row stays mounted");
        let reentrant_height = cell.frame().size.height;
        let reentrant_text = label_texts(&cell.contentView());
        let moved = reentrant_table
            .cellForRowAtIndexPath(&untouched_row)
            .expect("the second row stays mounted");
        let moved_height = moved.frame().size.height;
        assert!(
            (reentrant_height - short).abs() < 1.0,
            "the newest emission wins over the queued reentrant one: {reentrant_height} vs {short}"
        );
        assert!(
            reentrant_text.iter().any(|t| t == "reentrant row"),
            "the reentrant row shows the newest payload, got {reentrant_text:?}"
        );
        assert!(
            moved_height > short + 10.0,
            "the second row takes its own updated contract: {moved_height}"
        );
    }

    /// A row whose `View::body` removes the first item during
    /// materialization emits a reentrant membership change; rows
    /// realized while that emission is still queued must come from the
    /// retained snapshot — old ids and row data stay exact — and the
    /// next applied snapshot reflects the new membership. Reproduces
    /// the `render_row` live-read abort (`#307`/`#1418`).
    pub fn reentrant_remove_realizes_the_retained_snapshot() {
        let env = crate::resolve::env();
        let fired = Rc::new(Cell::new(false));
        let rendered = Rc::new(std::cell::RefCell::new(Vec::new()));
        let items = ReactiveList::from(vec![
            ProbeRow {
                id: 30,
                tall: false,
            },
            ProbeRow {
                id: 31,
                tall: false,
            },
            ProbeRow {
                id: 32,
                tall: false,
            },
        ]);
        let list = List::for_each(items.clone(), {
            let fired = Rc::clone(&fired);
            let rendered = Rc::clone(&rendered);
            move |row: ProbeRow| {
                rendered.borrow_mut().push((row.id, items.len()));
                let content: AnyView = if row.id == 30 {
                    AnyView::new(RemovingRow {
                        items: items.clone(),
                        fired: fired.clone(),
                    })
                } else {
                    AnyView::new(text("row"))
                };
                ListItem::new(content)
            }
        });
        let mount = waterui_apple::native_test_support::mount_uikit_in(
            mtm(),
            AnyView::new(list),
            &env,
            cocoa_ui::Rect::new(0.0, 0.0, 393.0, 852.0),
        );
        let remove_table = table(mount.content.view());
        // Pumping until the retained realizations land materializes the
        // rows: the body's `remove(0)` runs mid-pass, so every realization
        // after it must still read the retained snapshot. UIKit realizes
        // cells repeatedly, so the log interleaves — the decisive entries
        // are rows whose retained id no longer exists at the same live
        // position (id 30 while live is `[31, 32]`), which a live read
        // could never produce.
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                if remove_table.numberOfRowsInSection(0) != 2 {
                    return false;
                }
                let log = rendered.borrow();
                log.first() == Some(&(30, 3))
                    && [30_u64, 31, 32]
                        .iter()
                        .all(|id| log.iter().any(|&(entry, len)| entry == *id && len == 2))
            }),
            "the retained snapshot realizes every row while the removal applies"
        );
        {
            let log = rendered.borrow();
            assert_eq!(
                log.first(),
                Some(&(30, 3)),
                "row 0 realized before the reentrant removal: {log:?}"
            );
            for retained in [30_u64, 31, 32] {
                assert!(
                    log.iter()
                        .any(|&(id, live_len)| id == retained && live_len == 2),
                    "retained id {retained} realized after the live source had shrunk: {log:?}"
                );
            }
            assert!(
                log.iter().all(|&(id, _)| [30, 31, 32].contains(&id)),
                "every realization used retained membership: {log:?}"
            );
        }
        assert_eq!(
            remove_table.numberOfRowsInSection(0),
            2,
            "the applied snapshot reflects the removed first row"
        );
        let cell = remove_table
            .cellForRowAtIndexPath(&NSIndexPath::indexPathForRow_inSection(0, 0))
            .expect("the surviving first row stays mounted");
        let cell_text = label_texts(&cell.contentView());
        assert!(
            cell_text.iter().any(|t| t == "row"),
            "the re-materialized row shows its retained payload, got {cell_text:?}"
        );
    }

    /// Every `UILabel` text anywhere under `view` — the observable
    /// payload a mounted leaf settles to once queued work drains.
    fn label_texts(view: &PlatformView) -> Vec<String> {
        descendants(view)
            .iter()
            .filter_map(|view| {
                view.downcast_ref::<UILabel>()
                    .and_then(UILabel::text)
                    .map(|text| text.to_string())
            })
            .collect()
    }

    /// The mounted cell's height at `index` — `None` while the row has
    /// not materialized.
    fn cell_height(table: &UITableView, index: &NSIndexPath) -> Option<f64> {
        table
            .cellForRowAtIndexPath(index)
            .map(|cell| cell.frame().size.height)
    }

    /// The mounted cell at `index` shows `needle` in one of its labels.
    fn cell_shows(table: &UITableView, index: &NSIndexPath, needle: &str) -> bool {
        table.cellForRowAtIndexPath(index).is_some_and(|cell| {
            label_texts(&cell.contentView())
                .iter()
                .any(|text| text.contains(needle))
        })
    }

    /// `row`-prefixed `UILabel` texts anywhere under `view`, sorted —
    /// membership evidence for the container/table reentrant probes.
    fn row_labels(view: &PlatformView) -> Vec<String> {
        let mut labels: Vec<String> = label_texts(view)
            .into_iter()
            .filter(|text| text.starts_with("row "))
            .collect();
        labels.sort();
        labels
    }

    /// A generator that removes a member while deferred materialization
    /// holds the container borrow emits a reentrant contents
    /// notification — it must record and deliver at the next safe
    /// boundary, never collide with the in-flight borrow (`#319`).
    pub fn reentrant_container_remove_applies_at_boundary() {
        use waterui::component::lazy::Lazy;
        use waterui::views::ForEach;

        let env = crate::resolve::env();
        let fired = Rc::new(Cell::new(false));
        let items = ReactiveList::from(vec![
            ProbeRow {
                id: 40,
                tall: false,
            },
            ProbeRow {
                id: 41,
                tall: false,
            },
            ProbeRow {
                id: 42,
                tall: false,
            },
        ]);
        let container = Lazy::vstack(ForEach::new(items.clone(), {
            move |row: ProbeRow| {
                if !fired.replace(true) {
                    let _ = items.remove(0);
                }
                text(format!("row {}", row.id))
            }
        }));
        let mount = waterui_apple::native_test_support::mount_uikit_in(
            mtm(),
            AnyView::new(container),
            &env,
            cocoa_ui::Rect::new(0.0, 0.0, 393.0, 852.0),
        );
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                row_labels(mount.content.view()) == ["row 41", "row 42"]
            }),
            "the reentrant emission applies once the borrow releases"
        );
        let labels = row_labels(mount.content.view());
        assert_eq!(
            labels,
            ["row 41", "row 42"],
            "the reentrant emission applies once the borrow releases"
        );
    }

    /// Rows materialized while the table holds its borrow whose generator
    /// mutates the collection emit a reentrant rows notification — it
    /// must record and deliver at the next safe boundary (`#319`). The
    /// first generator call fires during the initial sync before the
    /// column lands, so its emission must also survive that window.
    pub fn reentrant_table_rows_apply_at_boundary() {
        use waterui::component::table::{col, table};
        use waterui::views::ForEach;

        let env = crate::resolve::env();
        let first = Rc::new(Cell::new(false));
        let second = Rc::new(Cell::new(false));
        let items = ReactiveList::from(vec![
            ProbeRow {
                id: 50,
                tall: false,
            },
            ProbeRow {
                id: 51,
                tall: false,
            },
        ]);
        let rows = ForEach::new(items.clone(), {
            let items = items.clone();
            let first = Rc::clone(&first);
            let second = Rc::clone(&second);
            move |row: ProbeRow| {
                if !first.replace(true) {
                    items.push(ProbeRow {
                        id: 52,
                        tall: false,
                    });
                } else if row.id == 53 && !second.replace(true) {
                    items.push(ProbeRow {
                        id: 54,
                        tall: false,
                    });
                }
                text(format!("row {}", row.id))
            }
        });
        let view = table(vec![col("c", rows)]);
        let mount = waterui_apple::native_test_support::mount_uikit_in(
            mtm(),
            AnyView::new(view),
            &env,
            cocoa_ui::Rect::new(0.0, 0.0, 393.0, 852.0),
        );
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                row_labels(mount.content.view()) == ["row 50", "row 51", "row 52"]
            }),
            "the first reentrant rows emission applies once the borrow releases"
        );
        // The post-mount apply runs under the table borrow; the row-53
        // generator emits reentrantly inside it.
        items.push(ProbeRow {
            id: 53,
            tall: false,
        });
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                row_labels(mount.content.view())
                    == ["row 50", "row 51", "row 52", "row 53", "row 54"]
            }),
            "both reentrant emissions apply once the borrow releases"
        );
        let labels = row_labels(mount.content.view());
        assert_eq!(
            labels,
            ["row 50", "row 51", "row 52", "row 53", "row 54"],
            "both reentrant emissions apply once the borrow releases"
        );
    }

    /// A rows apply whose generator mutates a column's label binding and
    /// the columns signal itself: both notifications land inside the
    /// in-flight apply — the coordinator must record them and deliver at
    /// the outermost finish (`#319`), reconciling the new column set
    /// inside the emission's own animation and draining the recorded
    /// reload with the transaction.
    pub fn reentrant_columns_and_label_apply_at_boundary() {
        use core::cell::OnceCell;
        use waterui::component::table::{TableColumn, col, table};
        use waterui::reactive::binding;
        use waterui::views::ForEach;

        let env = crate::resolve::env();
        let fired = Rc::new(Cell::new(false));
        let cross = Rc::new(Cell::new(false));
        let items = ReactiveList::from(vec![ProbeRow {
            id: 60,
            tall: false,
        }]);
        let other = ReactiveList::from(vec![ProbeRow {
            id: 70,
            tall: false,
        }]);
        let header = binding("h1".to_string());
        // The generator needs the column it lives in to keep it through
        // the reconcile; `TableColumn` clones share its `semantic_id`.
        let first_column = Rc::new(OnceCell::<TableColumn>::new());
        let columns = binding(Vec::<TableColumn>::new());
        let rows = ForEach::new(items.clone(), {
            let items = items.clone();
            let header = header.clone();
            let columns = columns.clone();
            let first_column = Rc::clone(&first_column);
            let fired = Rc::clone(&fired);
            let cross = Rc::clone(&cross);
            move |row: ProbeRow| {
                if row.id == 61 && !fired.replace(true) {
                    columns.set(vec![
                        first_column.get().expect("column installed").clone(),
                        col(
                            text(header.clone()),
                            ForEach::new(other.clone(), {
                                let items = items.clone();
                                let cross = Rc::clone(&cross);
                                move |row: ProbeRow| {
                                    // Materializing the joining column
                                    // mutates the first column's rows —
                                    // a cross-column emission recorded
                                    // mid-drain must still deliver.
                                    if !cross.replace(true) {
                                        items.push(ProbeRow {
                                            id: 62,
                                            tall: false,
                                        });
                                    }
                                    text(format!("row {}", row.id))
                                }
                            }),
                        ),
                    ]);
                    header.set("h2".to_string());
                }
                text(format!("row {}", row.id))
            }
        });
        let column = col(text(header), rows);
        let _ = first_column.set(column.clone());
        columns.set(vec![column]);
        let view = table(columns);
        let mount = waterui_apple::native_test_support::mount_uikit_in(
            mtm(),
            AnyView::new(view),
            &env,
            cocoa_ui::Rect::new(0.0, 0.0, 393.0, 852.0),
        );
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                row_labels(mount.content.view()) == ["row 60"]
            }),
            "the initial column rows materialize"
        );
        // The post-mount apply runs under the table borrow; the row-61
        // generator emits the label and columns notifications reentrantly
        // inside it.
        items.push(ProbeRow {
            id: 61,
            tall: false,
        });
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                row_labels(mount.content.view()) == ["row 60", "row 61", "row 62", "row 70"]
            }),
            "the recorded emissions deliver at the boundary"
        );
        let labels = row_labels(mount.content.view());
        assert_eq!(
            labels,
            ["row 60", "row 61", "row 62", "row 70"],
            "the recorded columns reconcile, cross-column, and label reload deliver at the boundary"
        );
    }
}

mod colors {
    use objc2_core_graphics::{CGColor, CGColorSpace, kCGColorSpaceExtendedLinearDisplayP3};
    use waterui::graphics::color::{Color, Working, WorkingColor, signal_color, srgb_to_linear};
    use waterui::reactive::binding;
    use waterui_backend_core::AnyView;

    #[cfg(target_os = "macos")]
    pub fn native_well_round_trip() {
        use cocoa_ui::objc2_app_kit::NSColorWell;
        use waterui::Signal;
        use waterui::component::form::picker::color::ColorPicker;

        let env = crate::resolve::env();
        let components = [0.9, 0.4, 0.1, 0.6];
        let source = binding(Color::new(Working(WorkingColor::new(components))));
        let leaf = waterui_apple::dispatch::render(
            AnyView::new(ColorPicker::new("Color", &source).with_alpha().with_hdr()),
            &env,
        );
        let well = cocoa_ui::view::subviews(leaf.view())
            .into_iter()
            .find_map(|view| view.downcast::<NSColorWell>().ok())
            .expect("the native color picker owns an NSColorWell");
        let color = well.color();
        well.setColor(&color);
        // SAFETY: the backend installed this selector on this retained target;
        // both remain owned by the live leaf on the actual main thread.
        assert!(unsafe { well.sendAction_to(well.action(), well.target().as_deref()) });
        let actual = source.snapshot().resolve(&env).snapshot();
        for (actual, expected) in actual.components.into_iter().zip(components) {
            assert!(
                (actual - expected).abs() < 1e-3,
                "native well round trip changed {expected} to {actual}"
            );
        }
    }

    fn assert_fill(view: &cocoa_ui::PlatformView, expected: [f32; 4]) {
        #[cfg(target_os = "macos")]
        let color = {
            view.layer()
                .expect("the color leaf has a backing layer")
                .backgroundColor()
                .expect("the color leaf has a fill")
        };
        #[cfg(target_os = "ios")]
        let color = {
            let background = view.backgroundColor().expect("the color leaf has a fill");
            // SAFETY: the retained UIKit color is read on the actual main thread.
            unsafe { background.CGColor() }
        };
        assert_eq!(CGColor::number_of_components(Some(&color)), 4);
        let space = CGColor::color_space(Some(&color)).expect("the fill has a color space");
        let name = CGColorSpace::name(Some(&space)).expect("the fill space is named");
        // SAFETY: CoreGraphics provides this immutable color-space identifier.
        assert_eq!(&*name, unsafe { kCGColorSpaceExtendedLinearDisplayP3 });
        // SAFETY: the retained color owns exactly four components, checked above.
        let actual = unsafe { std::slice::from_raw_parts(CGColor::components(Some(&color)), 4) };
        for (actual, expected) in actual.iter().zip(expected) {
            assert!(
                (actual - f64::from(expected)).abs() < 1e-4,
                "native channel {actual} differs from straight working channel {expected}"
            );
        }
    }

    pub fn native_hdr_and_updates() {
        let env = crate::resolve::env();
        let initial = [1.4, -0.2, 0.3, 0.4];
        let source = binding(Color::new(Working(WorkingColor::new(initial))));
        let leaf =
            waterui_apple::dispatch::render(AnyView::new(signal_color(source.clone())), &env);
        assert_fill(leaf.view(), initial);
        for components in [[1.4, 0.25, 0.125, 0.5], [0.9, 0.4, 0.1, 1.0]] {
            source.set(Color::new(Working(WorkingColor::new(components))));
            assert_fill(leaf.view(), components);
        }
    }

    pub fn srgb_known_values() {
        for (input, expected) in [
            (0.0, 0.0),
            (1.0, 1.0),
            (0.5, 0.214_041_14),
            (0.040_45, 0.040_45 / 12.92),
        ] {
            assert!((srgb_to_linear(input) - expected).abs() < 1e-7);
        }
    }
}
