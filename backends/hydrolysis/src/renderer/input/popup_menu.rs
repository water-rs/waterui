use super::*;
use core::ops::RangeInclusive;
use core::time::Duration;
use waterui::form::Calendar;
use waterui::shape::{RoundedRectangle, ShapeExt as _};
use waterui::theme::color::Surface;
use waterui_backend_core::widget::PickerMetrics;
use waterui_controls::label::LabelDisplayMode;
use waterui_controls::{Stepper, button, stepper::stepper};
use waterui_core::{AnimationExt as _, id::Id};
use waterui_form::picker::PickerStyle;
use waterui_form::picker::date::{Date, DatePickerType, DateTime};
use waterui_layout::frame::Frame;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::spacer::spacer;
use waterui_layout::stack::hstack;
use waterui_text::text;

#[derive(Clone)]
pub(crate) struct ContextMenuTarget {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) depth: usize,
    pub(crate) order: usize,
    pub(crate) items: nami::Computed<Vec<ResolvedMenuItem>>,
}

#[derive(Clone)]
pub(crate) enum PopupMenuNode {
    Command {
        label: SemanticLabel,
        plain_label: String,
        action: SharedAction<()>,
        disabled: bool,
    },
    Divider,
    Menu {
        label: SemanticLabel,
        plain_label: String,
        items: Vec<PopupMenuNode>,
    },
}

#[derive(Clone)]
pub(crate) struct PopupMenuStateGroup(pub(crate) Rc<RefCell<Vec<Binding<WindowState>>>>);

#[derive(Default)]
pub(crate) struct PopupMenuState {
    pub(crate) active_popup_menu_group: Option<PopupMenuStateGroup>,
    /// Open/closed handles for the picker menus currently in the render tree, each
    /// owned by its picker node. This is only a registry for "dismiss every menu"
    /// (outside click) — it is Rc-pruned, never flush-order-indexed, so a dropped
    /// picker's handle falls out by strong count and the live ones are deduplicated.
    pub(crate) node_picker_menus: Vec<Rc<Cell<bool>>>,
}

#[derive(Clone)]
pub(crate) struct PickerMenuEntry {
    pub(crate) label: String,
    pub(crate) tag: Id,
}

pub(crate) struct PickerMenuRequest {
    pub(crate) entries: Vec<PickerMenuEntry>,
    pub(crate) selection: Binding<Id>,
    pub(crate) open: Rc<Cell<bool>>,
    pub(crate) origin: LayoutPoint,
    pub(crate) width: f64,
    pub(crate) row_height: f64,
    pub(crate) selected: Id,
}

impl PopupMenuState {
    /// Register a picker node's open handle for "dismiss every menu". Prunes handles
    /// of dropped pickers (the registry is then their only holder) and deduplicates,
    /// so a node re-registering its handle on every flush is idempotent.
    pub(crate) fn register_picker_menu(&mut self, open: &Rc<Cell<bool>>) {
        self.node_picker_menus
            .retain(|handle| Rc::strong_count(handle) > 1);
        if !self
            .node_picker_menus
            .iter()
            .any(|handle| Rc::ptr_eq(handle, open))
        {
            self.node_picker_menus.push(Rc::clone(open));
        }
    }

    /// Close every open picker menu (an outside click dismisses them all).
    pub(crate) fn close_all_picker_menus(&mut self) {
        self.node_picker_menus
            .retain(|handle| Rc::strong_count(handle) > 1);
        for handle in &self.node_picker_menus {
            handle.set(false);
        }
    }
}

impl PopupMenuStateGroup {
    pub(crate) fn new() -> Self {
        Self(Rc::new(RefCell::new(Vec::new())))
    }

    pub(crate) fn push(&self, state: Binding<WindowState>) {
        self.0.borrow_mut().push(state);
    }

    pub(crate) fn truncate(&self, len: usize) {
        let mut states = self.0.borrow_mut();
        for state in states.drain(len..) {
            state.set(WindowState::Closed);
        }
    }

    pub(crate) fn close_all(&self) {
        self.truncate(0);
    }
}

impl_extractor!(PopupMenuStateGroup);

fn popup_window_origin(origin: LayoutPoint, env: &Environment) -> LayoutPoint {
    let window_origin = env
        .get::<HydrolysisWindowOrigin>()
        .copied()
        .expect("hydrolysis popup windows require HydrolysisWindowOrigin in environment");
    LayoutPoint::new(window_origin.x + origin.x, window_origin.y + origin.y)
}

fn popup_enter_animation() -> Animation {
    Animation::bezier(Duration::from_millis(120), 0.2, 0.0, 0.0, 1.0)
}

fn animated_popup_panel(content: impl View, group: PopupMenuStateGroup) -> impl View {
    let opacity = Binding::f32(0.0);
    let scale = Binding::f32(0.96);
    let enter_animation = popup_enter_animation();
    content
        .opacity(opacity.clone().with_animation(enter_animation.clone()))
        .scale(
            scale.clone().with_animation(enter_animation.clone()),
            scale.clone().with_animation(enter_animation),
        )
        .on_appear(move || {
            opacity.set(0.96);
            scale.set(1.0);
        })
        .with(group)
}

pub(crate) fn popup_menu_size(
    nodes: &[PopupMenuNode],
    metrics: TextContextMenuMetrics,
) -> (f64, f64) {
    let max_label_chars = nodes
        .iter()
        .filter_map(|node| match node {
            PopupMenuNode::Command { plain_label, .. }
            | PopupMenuNode::Menu { plain_label, .. } => Some(plain_label.chars().count()),
            PopupMenuNode::Divider => None,
        })
        .max()
        .unwrap_or(0) as f64;
    let width = (metrics.horizontal_padding * 2.0 + max_label_chars * metrics.width_per_char)
        .clamp(metrics.min_width, metrics.max_width);
    let height = (nodes.len() as f64 * metrics.row_height).max(metrics.row_height);
    (width, height)
}

pub(crate) fn popup_menu_window(
    nodes: Vec<PopupMenuNode>,
    origin: LayoutPoint,
    group: PopupMenuStateGroup,
    depth: usize,
    metrics: TextContextMenuMetrics,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let (width, height) = popup_menu_size(&nodes, metrics);
    let popup_origin_x = origin.x;
    let popup_origin_y = origin.y;
    let group_for_content = group.clone();
    let state_for_content = state.clone();
    let nodes_for_content = nodes.clone();
    let popup_content = move || {
        let mut rows = Vec::with_capacity(nodes_for_content.len());
        for (index, node) in nodes_for_content.clone().into_iter().enumerate() {
            match node {
                PopupMenuNode::Command {
                    label,
                    action,
                    disabled,
                    ..
                } => {
                    let button = Button::new(label).style(ButtonStyle::Borderless).action(
                        move |group: PopupMenuStateGroup, env: Environment| {
                            if disabled {
                                return;
                            }
                            group.close_all();
                            call_action_discarding_result(&action, &env);
                        },
                    );
                    rows.push(AnyView::new(button));
                }
                PopupMenuNode::Divider => rows.push(AnyView::new(Divider)),
                PopupMenuNode::Menu { label, items, .. } => {
                    let next_depth = depth + 1;
                    let child_origin = LayoutPoint::new(
                        popup_origin_x + width as f32,
                        popup_origin_y + (metrics.row_height * index as f64) as f32,
                    );
                    let button = Button::new(label).style(ButtonStyle::Borderless).action(
                        move |group: PopupMenuStateGroup, env: Environment| {
                            if items.is_empty() {
                                return;
                            }
                            group.truncate(next_depth);
                            let (window, child_state) = popup_menu_window(
                                items.clone(),
                                child_origin,
                                group.clone(),
                                next_depth,
                                metrics,
                            );
                            group.push(child_state);
                            env.get::<WindowManager>()
                                .expect(
                                    "hydrolysis popup menus require WindowManager in environment",
                                )
                                .show(window);
                        },
                    );
                    rows.push(AnyView::new(button));
                }
            }
        }
        let menu_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
            rows.into_iter().collect();
        AnyView::new(animated_popup_panel(
            menu_content
                .alignment(HorizontalAlignment::Leading)
                .spacing(0.0)
                .background(
                    RoundedRectangle::new((metrics.corner_radius / metrics.min_width) as f32)
                        .fill(waterui::Color::new(Surface)),
                ),
            group_for_content.clone(),
        ))
    };
    let mut popup = Window::new(
        TEXT_CONTEXT_MENU_WINDOW_TITLE,
        state_for_content,
        popup_content,
    )
    .style(WindowStyle::Borderless)
    .resizable(false)
    .background(Color::transparent());
    popup.closable = false;
    popup.frame.set(LayoutRect::new(
        origin,
        LayoutSize::new(width as f32, height as f32),
    ));
    (popup, state)
}

#[allow(
    clippy::too_many_arguments,
    reason = "fully specifies a popup-menu window layout; grouping into a struct would not improve clarity"
)]
pub(crate) fn picker_menu_window(
    entries: Vec<PickerMenuEntry>,
    selection: Binding<Id>,
    open: Rc<Cell<bool>>,
    origin: LayoutPoint,
    width: f64,
    row_height: f64,
    selected: Id,
    group: PopupMenuStateGroup,
    metrics: PickerMetrics,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let height = row_height * entries.len() as f64;
    let state_for_content = state.clone();
    let group_for_content = group.clone();
    let entries_for_content = entries.clone();
    let popup_content =
        move || {
            let mut rows = Vec::with_capacity(entries_for_content.len());
            for entry in entries_for_content.clone() {
                let label = entry.label.clone();
                let target = entry.tag;
                let row_selection = selection.clone();
                let row_group = group_for_content.clone();
                let row_open = Rc::clone(&open);
                let row = Frame::new(button(label).style(ButtonStyle::Borderless).action(
                    move || {
                        if row_selection.get() != target {
                            row_selection.set(target);
                        }
                        row_open.set(false);
                        row_group.close_all();
                    },
                ))
                .width(width as f32)
                .height(row_height as f32);
                if target == selected {
                    rows.push(AnyView::new(row.background(
                        RoundedRectangle::new(0.0).fill(Color::new(Surface).with_opacity(0.84)),
                    )));
                } else {
                    rows.push(AnyView::new(row));
                }
            }
            let menu_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
                rows.into_iter().collect();
            let panel = menu_content
                .alignment(HorizontalAlignment::Leading)
                .spacing(0.0)
                .background(
                    RoundedRectangle::new((metrics.popup_corner_radius / width) as f32)
                        .fill(Color::new(Surface).with_opacity(0.96)),
                );
            AnyView::new(animated_popup_panel(panel, group_for_content.clone()))
        };
    let mut popup = Window::new("WaterUI Picker Menu", state_for_content, popup_content)
        .style(WindowStyle::Borderless)
        .resizable(false)
        .background(Color::transparent());
    popup.closable = false;
    popup.frame.set(LayoutRect::new(
        origin,
        LayoutSize::new(width as f32, height as f32),
    ));
    (popup, state)
}

fn color_picker_palette() -> [(&'static str, Color); 12] {
    [
        ("Red", Color::srgb(0xba, 0x1a, 0x1a)),
        ("Orange", Color::srgb(0xc2, 0x41, 0x0c)),
        ("Amber", Color::srgb(0x8a, 0x5a, 0x00)),
        ("Green", Color::srgb(0x2e, 0x7d, 0x32)),
        ("Teal", Color::srgb(0x00, 0x79, 0x6b)),
        ("Cyan", Color::srgb(0x00, 0x6d, 0x8f)),
        ("Blue", Color::srgb(0x0b, 0x57, 0xd0)),
        ("Indigo", Color::srgb(0x44, 0x38, 0xca)),
        ("Purple", Color::srgb(0x7b, 0x1f, 0xa2)),
        ("Pink", Color::srgb(0xa0, 0x18, 0x55)),
        ("Brown", Color::srgb(0x79, 0x55, 0x48)),
        ("Grey", Color::srgb(0x5f, 0x63, 0x68)),
    ]
}

pub(crate) fn color_picker_window(
    value: Binding<Color>,
    support_alpha: bool,
    support_hdr: bool,
    origin: LayoutPoint,
    group: PopupMenuStateGroup,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let width = 280.0;
    let swatch = 40.0;
    let gap = 8.0;
    let rows = 3.0;
    let alpha_row_height = if support_alpha { 48.0 } else { 0.0 };
    let hdr_row_height = if support_hdr { 48.0 } else { 0.0 };
    let height = 16.0 + rows * swatch + 2.0 * gap + alpha_row_height + hdr_row_height + 16.0;
    let group_for_content = group.clone();
    let state_for_content = state.clone();
    let popup_content = move || {
        let palette = color_picker_palette();
        let mut row_views = Vec::with_capacity(5);
        for row_index in 0..3 {
            let mut swatches = Vec::with_capacity(4);
            for column_index in 0..4 {
                let palette_index = row_index * 4 + column_index;
                let (label, color) = palette[palette_index].clone();
                let selected = value.clone();
                let group = group_for_content.clone();
                let swatch_color = color.clone();
                swatches.push(AnyView::new(
                    Frame::new(
                        button(label)
                            .style(ButtonStyle::Borderless)
                            .action(move || {
                                selected.set(color.clone());
                                group.close_all();
                            })
                            .install(LabelDisplayMode::Hidden)
                            .background(RoundedRectangle::new(0.2).fill(swatch_color)),
                    )
                    .width(swatch as f32)
                    .height(swatch as f32),
                ));
            }
            let row: waterui_layout::stack::HStack<(Vec<AnyView>,)> =
                swatches.into_iter().collect();
            row_views.push(AnyView::new(row.spacing(gap)));
        }

        if support_alpha {
            let selected = value.clone();
            let group = group_for_content.clone();
            row_views.push(AnyView::new(
                Frame::new(button("50% opacity").style(ButtonStyle::Borderless).action(
                    move || {
                        let current = selected.get();
                        selected.set(current.with_opacity(0.5));
                        group.close_all();
                    },
                ))
                .width((width - 32.0) as f32)
                .height(40.0),
            ));
        }

        if support_hdr {
            let selected = value.clone();
            let group = group_for_content.clone();
            row_views.push(AnyView::new(
                Frame::new(
                    button("HDR headroom")
                        .style(ButtonStyle::Borderless)
                        .action(move || {
                            let current = selected.get();
                            selected.set(current.with_headroom(1.0));
                            group.close_all();
                        }),
                )
                .width((width - 32.0) as f32)
                .height(40.0),
            ));
        }

        let content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
            row_views.into_iter().collect();
        let panel = content
            .alignment(HorizontalAlignment::Leading)
            .spacing(gap)
            .padding_with(EdgeInsets::all(16.0))
            .background(RoundedRectangle::new(0.05).fill(Color::new(Surface).with_opacity(0.96)));
        AnyView::new(animated_popup_panel(panel, group_for_content.clone()))
    };
    let mut popup = Window::new("WaterUI Color Picker", state_for_content, popup_content)
        .style(WindowStyle::Borderless)
        .resizable(false)
        .background(Color::transparent());
    popup.closable = false;
    popup.frame.set(LayoutRect::new(
        origin,
        LayoutSize::new(width as f32, height as f32),
    ));
    (popup, state)
}

fn time_part_binding(
    value: &Binding<i32>,
    range: RangeInclusive<i32>,
    label: &'static str,
) -> Stepper {
    stepper(label, value)
        .range(range)
        .value_formatter(|part| format!("{part:02}"))
}

fn apply_staged_date_time(
    value: &Binding<DateTime>,
    range: &RangeInclusive<DateTime>,
    date: Date,
    hour: i32,
    minute: i32,
    second: i32,
) {
    let hour = i8::try_from(hour).expect("date picker staged hour must fit i8");
    let minute = i8::try_from(minute).expect("date picker staged minute must fit i8");
    let second = i8::try_from(second).expect("date picker staged second must fit i8");
    let current = value.get();
    let time = current.time();
    let next = date
        .at(hour, minute, second, time.subsec_nanosecond())
        .clamp(*range.start(), *range.end());
    value.set(next);
}

pub(crate) fn date_picker_window(
    value: Binding<DateTime>,
    range: RangeInclusive<DateTime>,
    ty: DatePickerType,
    origin: LayoutPoint,
    group: PopupMenuStateGroup,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let current = value.get().clamp(*range.start(), *range.end());
    let staged_date = Binding::container(current.date());
    let staged_visible_month = Binding::container(current.date());
    let current_time = current.time();
    let staged_hour = Binding::container(i32::from(current_time.hour()));
    let staged_minute = Binding::container(i32::from(current_time.minute()));
    let staged_second = Binding::container(i32::from(current_time.second()));
    let uses_date = matches!(
        ty,
        DatePickerType::Date
            | DatePickerType::DateHourAndMinute
            | DatePickerType::DateHourMinuteAndSecond
    );
    let uses_time = !matches!(ty, DatePickerType::Date);
    let uses_second = matches!(
        ty,
        DatePickerType::HourMinuteAndSecond | DatePickerType::DateHourMinuteAndSecond
    );
    let width = 360.0;
    let height = if uses_date && uses_time {
        520.0
    } else if uses_date {
        430.0
    } else {
        260.0
    };
    let range_start = *range.start();
    let range_end = *range.end();
    let group_for_content = group.clone();
    let state_for_content = state.clone();
    let popup_content = move || {
        let mut sections = Vec::new();
        sections.push(AnyView::new(text("Select date").headline()));
        if uses_date {
            let visible_month = staged_visible_month.clone();
            sections.push(AnyView::new(
                Calendar::new("Date", &staged_date, &visible_month)
                    .range(range_start.date()..=range_end.date())
                    .hide_label(),
            ));
        }
        if uses_time {
            let mut time_controls = Vec::new();
            time_controls.push(AnyView::new(time_part_binding(
                &staged_hour,
                0..=23,
                "Hour",
            )));
            time_controls.push(AnyView::new(time_part_binding(
                &staged_minute,
                0..=59,
                "Minute",
            )));
            if uses_second {
                time_controls.push(AnyView::new(time_part_binding(
                    &staged_second,
                    0..=59,
                    "Second",
                )));
            }
            let time_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
                time_controls.into_iter().collect();
            sections.push(AnyView::new(time_content.spacing(8.0)));
        }
        let cancel_group = group_for_content.clone();
        let apply_group = group_for_content.clone();
        let apply_value = value.clone();
        let apply_range = range.clone();
        let apply_date = staged_date.clone();
        let apply_hour = staged_hour.clone();
        let apply_minute = staged_minute.clone();
        let apply_second = staged_second.clone();
        sections.push(AnyView::new(
            hstack((
                spacer(),
                button("Cancel")
                    .style(ButtonStyle::Borderless)
                    .action(move || {
                        cancel_group.close_all();
                    }),
                button("OK").style(ButtonStyle::Borderless).action(move || {
                    apply_staged_date_time(
                        &apply_value,
                        &apply_range,
                        apply_date.get(),
                        apply_hour.get(),
                        apply_minute.get(),
                        if uses_second { apply_second.get() } else { 0 },
                    );
                    apply_group.close_all();
                }),
            ))
            .spacing(8.0),
        ));

        let content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
            sections.into_iter().collect();
        let panel = content
            .alignment(HorizontalAlignment::Leading)
            .spacing(16.0)
            .padding_with(EdgeInsets::all(24.0))
            .background(RoundedRectangle::new(0.04).fill(Color::new(Surface).with_opacity(0.96)));
        AnyView::new(animated_popup_panel(panel, group_for_content.clone()))
    };
    let mut popup = Window::new("WaterUI Date Picker", state_for_content, popup_content)
        .style(WindowStyle::Borderless)
        .resizable(false)
        .background(Color::transparent());
    popup.closable = false;
    popup.frame.set(LayoutRect::new(
        origin,
        LayoutSize::new(width as f32, height as f32),
    ));
    (popup, state)
}

impl HydrolysisRenderer {
    pub(crate) fn active_popup_menu_visible(&self) -> bool {
        self.popup_menu.active_popup_menu_group.is_some()
    }

    pub(crate) fn dismiss_active_popup_menu(&mut self) {
        if let Some(group) = self.popup_menu.active_popup_menu_group.take() {
            group.close_all();
        }
        self.popup_menu.close_all_picker_menus();
    }

    pub(crate) fn register_picker_menu(&mut self, open: &Rc<Cell<bool>>) {
        self.popup_menu.register_picker_menu(open);
    }

    pub(crate) fn topmost_context_menu_target_at_point(
        &self,
        point: vello::kurbo::Point,
    ) -> Option<ContextMenuTarget> {
        self.hit_test
            .context_menu_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .max_by(|(left_index, left), (right_index, right)| {
                Self::target_hit_priority(left.depth, left.order, *left_index).cmp(
                    &Self::target_hit_priority(right.depth, right.order, *right_index),
                )
            })
            .map(|(_, target)| target.clone())
    }

    pub(crate) fn show_popup_menu_nodes(
        &mut self,
        nodes: Vec<PopupMenuNode>,
        origin: LayoutPoint,
        env: &Environment,
    ) -> bool {
        if nodes.is_empty() {
            return false;
        }
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        let metrics = widget_theme(env).text_context_menu_metrics();
        let popup_origin = popup_window_origin(origin, env);
        let (window, state) = popup_menu_window(nodes, popup_origin, group.clone(), 0, metrics);
        group.push(state);
        env.get::<WindowManager>()
            .expect("hydrolysis popup menus require WindowManager in environment")
            .show(window);
        self.popup_menu.active_popup_menu_group = Some(group);
        true
    }

    pub(crate) fn show_picker_menu(
        &mut self,
        request: PickerMenuRequest,
        env: &Environment,
    ) -> bool {
        if request.entries.is_empty() {
            return false;
        }
        self.dismiss_active_popup_menu();
        request.open.set(true);
        let group = PopupMenuStateGroup::new();
        let metrics = widget_theme(env).picker_metrics(PickerStyle::Menu);
        let popup_origin = popup_window_origin(request.origin, env);
        let (window, state) = picker_menu_window(
            request.entries,
            request.selection,
            request.open,
            popup_origin,
            request.width,
            request.row_height,
            request.selected,
            group.clone(),
            metrics,
        );
        group.push(state);
        env.get::<WindowManager>()
            .expect("hydrolysis picker menus require WindowManager in environment")
            .show(window);
        self.popup_menu.active_popup_menu_group = Some(group);
        self.request_refresh();
        true
    }

    pub(crate) fn show_color_picker(
        &mut self,
        value: Binding<Color>,
        support_alpha: bool,
        support_hdr: bool,
        origin: LayoutPoint,
        env: &Environment,
    ) -> bool {
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        let popup_origin = popup_window_origin(origin, env);
        let (window, state) = color_picker_window(
            value,
            support_alpha,
            support_hdr,
            popup_origin,
            group.clone(),
        );
        group.push(state);
        env.get::<WindowManager>()
            .expect("hydrolysis color picker requires WindowManager in environment")
            .show(window);
        self.popup_menu.active_popup_menu_group = Some(group);
        true
    }

    pub(crate) fn show_date_picker(
        &mut self,
        value: Binding<DateTime>,
        range: RangeInclusive<DateTime>,
        ty: DatePickerType,
        origin: LayoutPoint,
        env: &Environment,
    ) -> bool {
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        let popup_origin = popup_window_origin(origin, env);
        let (window, state) = date_picker_window(value, range, ty, popup_origin, group.clone());
        group.push(state);
        env.get::<WindowManager>()
            .expect("hydrolysis date picker requires WindowManager in environment")
            .show(window);
        self.popup_menu.active_popup_menu_group = Some(group);
        true
    }

    pub(crate) fn register_context_menu_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        items: nami::Computed<Vec<ResolvedMenuItem>>,
    ) {
        self.register_context_menu_target_data(bounds, items, self.render_depth);
    }

    pub(crate) fn register_context_menu_target_data(
        &mut self,
        bounds: vello::kurbo::Rect,
        items: nami::Computed<Vec<ResolvedMenuItem>>,
        depth: usize,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.hit_test.next_hit_test_order();
        self.hit_test.context_menu_targets.push(ContextMenuTarget {
            bounds,
            depth,
            order,
            items,
        });
    }
}
