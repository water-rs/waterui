//! `.material_group()` (water-rs/waterui#1999): the `IgnorableMetadata`
//! wrapper opens a material scope the flush keeps on a stack, and the mount
//! table's group key `(scope, within-window level, resolved colour scheme,
//! install canvas)` makes the members under one scope share one
//! `cherenkov::BackdropGroup` — one capture, one chain. A member outside
//! every group is a group of its own; members of different levels, schemes,
//! canvases, or scopes never share.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

#[cfg(feature = "accessibility")]
use accesskit::Role;
#[cfg(feature = "accessibility")]
use nami::collection::SignalCollection;
use nami::{Binding, Computed, SignalExt as _};
use waterui::FilterViewExt as _;
use waterui::ViewExt as _;
use waterui::background::Material;
#[cfg(feature = "accessibility")]
use waterui::component::list::{List, ListItem};
use waterui::graphics::Color;
use waterui::metadata::anchored_overlay::{AnchorEdge, AnchoredOverlay};
use waterui::prelude::text;
use waterui::theme::ColorScheme;
use waterui::widget::condition::when;
use waterui_controls::button::button;
use waterui_core::AnyView;
use waterui_core::env::Store;
use waterui_core::extract::Use;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::scroll;
use waterui_layout::stack::{VStack, hstack, vstack, zstack};

use super::{
    MinimalTestTheme, capture_bytes, material_layers, mounts, pumped_test_environment,
    test_environment,
};
use crate::HeadlessRuntime;
use crate::engine::WidgetTheme;
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::renderer::mount::backdrop::BackdropScope;
use crate::renderer::{
    FontFamilyResolution, HydroState, ProposalSize, measure_view_dimensions_with_proposal,
    normalize_layout_view, view_renders_nothing,
};
use crate::text::SessionTextEngine;

/// The window, in points.
const WIDTH: u32 = 160;
const HEIGHT: u32 = 120;
/// Each member's size, in points — small enough for two to overlap exactly.
const MEMBER: (f32, f32) = (80.0, 60.0);
/// Device pixels per point.
const DISPLAY_SCALE: f64 = 2.0;

/// One `level` material member, `MEMBER` points large.
fn member(level: Material) -> AnyView {
    AnyView::new(().size(MEMBER.0, MEMBER.1).background(level))
}

/// A member and a label side by side — the lazy stack's row.
fn row() -> impl waterui::View {
    hstack((member(Material::Regular), text("row")))
}

/// `view` after one rendered frame at `DISPLAY_SCALE`.
fn rendered(view: impl Fn() -> AnyView + 'static) -> HeadlessRuntime {
    rendered_at_scale(view, DISPLAY_SCALE)
}

/// `view` after one rendered frame at `scale` device pixels per point.
fn rendered_at_scale(view: impl Fn() -> AnyView + 'static, scale: f64) -> HeadlessRuntime {
    let builder = AnyViewBuilder::<AnyView>::new(view);
    let mut runtime = HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        WIDTH,
        HEIGHT,
        MinimalTestTheme::default(),
    )
    .with_scale_factor(scale);
    let _ = runtime.pump_snapshot();
    runtime
}

/// The `.material_group()` scope `member` flushed under — the cell address
/// its key's `Scoped` identity carries — `None` outside every group.
fn member_scope(runtime: &HeadlessRuntime, member: cherenkov::LayerId) -> Option<usize> {
    match mounts(runtime).backdrop_scope(member)? {
        BackdropScope::Scoped(scope) => Some(scope),
        BackdropScope::Solo(_) => None,
    }
}

/// Two same-bounds `level` members under `red`, with `.material_group()`
/// applied when `grouped`.
fn two_members(level: Material, grouped: bool) -> impl Fn() -> AnyView {
    move || {
        let members = zstack((member(level), member(level)));
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            if grouped {
                AnyView::new(members.material_group())
            } else {
                AnyView::new(members)
            },
        )))
    }
}

/// One member alone at the members' bounds — the one-capture baseline.
fn solo() -> AnyView {
    AnyView::new(zstack((
        Color::srgb(230, 38, 38),
        member(Material::Regular),
    )))
}

fn pump(runtime: &mut HeadlessRuntime) {
    for _ in 0..64 {
        let _ = runtime.pump_at(false, Instant::now());
        if runtime.is_settled() {
            break;
        }
    }
}

/// A dark member and a light member at one level inside one
/// `.material_group()` share no capture: the resolved colour scheme is
/// part of the group key, so the frame holds two groups and each group's
/// chain runs its own scheme's tone.
#[test]
fn members_of_different_schemes_share_no_group() {
    let runtime = rendered(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            vstack((
                member(Material::Regular).with(Store::<ColorScheme, Computed<ColorScheme>>::new(
                    Computed::constant(ColorScheme::Dark),
                )),
                member(Material::Regular),
            ))
            .material_group(),
        )))
    });
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 2, "the frame presents both members");
    assert!(
        layers.iter().all(|layer| {
            member_scope(&runtime, *layer).is_some()
                && member_scope(&runtime, *layer) == member_scope(&runtime, layers[0])
        }),
        "both members flushed under the group's scope"
    );
    let mounts = mounts(&runtime);
    assert_eq!(mounts.backdrop_group_count(), 2);
    assert_ne!(
        mounts.backdrop_group_id(layers[0]),
        mounts.backdrop_group_id(layers[1]),
        "the two schemes key two groups"
    );
    assert_eq!(
        mounts.backdrop_scheme(layers[0]),
        Some(ColorScheme::Dark),
        "the first member's subtree installs Dark"
    );
    assert_eq!(
        mounts.backdrop_scheme(layers[1]),
        Some(ColorScheme::Light),
        "the second member resolves the window's Light"
    );
    assert_ne!(
        mounts.backdrop_tone(layers[0]),
        mounts.backdrop_tone(layers[1]),
        "each group's chain carries its own tone"
    );
}

/// Two `Regular` members under one `.material_group()` join one shared
/// backdrop group: the same group id on both mounts, one live group in the
/// table, and exactly one capture's bytes.
#[test]
fn materials_in_one_group_share_one_backdrop_group() {
    let runtime = rendered(two_members(Material::Regular, true));
    let solo = rendered(solo);
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 2, "the frame presents both members");
    assert!(
        layers.iter().all(|layer| {
            member_scope(&runtime, *layer).is_some()
                && member_scope(&runtime, *layer) == member_scope(&runtime, layers[0])
        }),
        "both members flushed under the group's scope"
    );
    let mounts = mounts(&runtime);
    let first = mounts.backdrop_group_id(layers[0]);
    let second = mounts.backdrop_group_id(layers[1]);
    assert!(first.is_some());
    assert_eq!(first, second, "the members sample one shared group");
    assert_eq!(mounts.backdrop_group_count(), 1);
    assert_eq!(
        capture_bytes(&runtime),
        capture_bytes(&solo),
        "two members in one group cost exactly one capture"
    );
}

/// Without the modifier each member is a backdrop group of its own.
#[test]
fn ungrouped_materials_get_a_group_each() {
    let runtime = rendered(two_members(Material::Regular, false));
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 2);
    assert!(
        layers
            .iter()
            .all(|layer| member_scope(&runtime, *layer).is_none()),
        "no member flushed under a group scope"
    );
    let mounts = mounts(&runtime);
    assert_ne!(
        mounts.backdrop_group_id(layers[0]),
        mounts.backdrop_group_id(layers[1]),
        "ungrouped members never share a capture"
    );
    assert_eq!(mounts.backdrop_group_count(), 2);
}

/// `Regular` and `Thick` members under one scope run different chains, so
/// they get a group each even though the scope is shared.
#[test]
fn different_levels_under_one_group_get_two_groups() {
    let runtime = rendered(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            vstack((member(Material::Regular), member(Material::Thick))).material_group(),
        )))
    });
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 2);
    assert_eq!(
        member_scope(&runtime, layers[0]),
        member_scope(&runtime, layers[1]),
        "both members flushed under the one group's scope"
    );
    let mounts = mounts(&runtime);
    assert_ne!(
        mounts.backdrop_group_id(layers[0]),
        mounts.backdrop_group_id(layers[1]),
        "the level is part of the group key"
    );
    assert_eq!(mounts.backdrop_group_count(), 2);
}

/// A nested `.material_group()` is its own scope: the members inside it do
/// not join the enclosing group's members.
#[test]
fn a_nested_group_starts_its_own_scope() {
    let runtime = rendered(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            vstack((
                member(Material::Regular),
                vstack((member(Material::Regular), member(Material::Regular))).material_group(),
            ))
            .material_group(),
        )))
    });
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 3);
    assert_eq!(
        member_scope(&runtime, layers[1]),
        member_scope(&runtime, layers[2]),
        "the inner members share the nested scope"
    );
    assert_ne!(
        member_scope(&runtime, layers[0]),
        member_scope(&runtime, layers[1]),
        "the outer member flushed under the enclosing scope"
    );
    let mounts = mounts(&runtime);
    assert_eq!(
        mounts.backdrop_group_id(layers[1]),
        mounts.backdrop_group_id(layers[2]),
        "the inner members share one group"
    );
    assert_ne!(
        mounts.backdrop_group_id(layers[0]),
        mounts.backdrop_group_id(layers[1]),
        "the outer group's member does not join the inner group"
    );
    assert_eq!(mounts.backdrop_group_count(), 2);
}

/// Materials share a capture only inside one compositing canvas: a member
/// inside a filtered view mounts under the filter's own canvas, so it does
/// not join the member outside even though both share one group scope.
#[test]
fn a_member_in_a_filtered_view_gets_its_own_group() {
    let runtime = rendered(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            vstack((
                member(Material::Regular).blur(2.0f32),
                member(Material::Regular),
            ))
            .material_group(),
        )))
    });
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 2);
    assert_eq!(
        member_scope(&runtime, layers[0]),
        member_scope(&runtime, layers[1]),
        "both members flushed under the one group's scope"
    );
    let mounts = mounts(&runtime);
    assert_ne!(
        mounts.backdrop_group_id(layers[0]),
        mounts.backdrop_group_id(layers[1]),
        "different install canvases never share a capture"
    );
    assert_eq!(mounts.backdrop_group_count(), 2);
}

/// A member leaving the frame drops out of the group's membership; the
/// group lives on for the member still presented, and releases at the
/// commit of the first frame with no visible member at all.
#[test]
fn the_group_survives_one_member_and_releases_with_the_last() {
    let first = Binding::container(true);
    let second = Binding::container(true);
    let (first_in_view, second_in_view) = (first.clone(), second.clone());
    let mut runtime = {
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(zstack((
                Color::srgb(230, 38, 38),
                vstack((
                    when(first_in_view.clone(), || member(Material::Regular)),
                    when(second_in_view.clone(), || member(Material::Regular)),
                ))
                .material_group(),
            )))
        });
        let mut runtime = HeadlessRuntime::new_for_tests(
            pumped_test_environment(),
            builder,
            WIDTH,
            HEIGHT,
            MinimalTestTheme::default(),
        )
        .with_scale_factor(DISPLAY_SCALE);
        pump(&mut runtime);
        runtime
    };

    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 2);
    let group = mounts(&runtime).backdrop_group_id(layers[0]);
    assert!(group.is_some());
    assert_eq!(mounts(&runtime).backdrop_group_count(), 1);

    first.set(false);
    pump(&mut runtime);
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 1, "one member left the frame");
    assert_eq!(
        mounts(&runtime).backdrop_group_id(layers[0]),
        group,
        "the remaining member keeps the shared group"
    );
    assert_eq!(mounts(&runtime).backdrop_group_count(), 1);

    second.set(false);
    pump(&mut runtime);
    assert_eq!(material_layers(&runtime), [] as [cherenkov::LayerId; 0]);
    assert_eq!(
        mounts(&runtime).backdrop_group_count(),
        0,
        "with no visible member the group releases"
    );
}

/// A display-scale change rebuilds the shared group and re-points every
/// member at it.
#[test]
fn a_display_scale_change_rebuilds_the_shared_group() {
    let mut runtime = rendered(two_members(Material::Regular, true));
    let layers = material_layers(&runtime);
    assert_eq!(
        layers
            .iter()
            .map(|layer| mounts(&runtime).backdrop_display_scale(*layer))
            .collect::<Vec<_>>(),
        vec![Some(DISPLAY_SCALE), Some(DISPLAY_SCALE)]
    );
    let before = mounts(&runtime).backdrop_group_id(layers[0]);
    assert!(before.is_some());

    runtime.set_scale_factor(1.0);
    pump(&mut runtime);

    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 2);
    let mounts = mounts(&runtime);
    let after = mounts.backdrop_group_id(layers[0]);
    assert_ne!(
        before, after,
        "the scale change rebuilt the group under a new id"
    );
    assert_eq!(
        after,
        mounts.backdrop_group_id(layers[1]),
        "the rebuilt group is still shared"
    );
    assert_eq!(mounts.backdrop_group_count(), 1);
    assert_eq!(
        layers
            .iter()
            .map(|layer| mounts.backdrop_display_scale(*layer))
            .collect::<Vec<_>>(),
        vec![Some(1.0), Some(1.0)],
        "both members follow the rebuilt group"
    );
    let solo_at_1x = rendered_at_scale(solo, 1.0);
    assert_eq!(
        capture_bytes(&runtime),
        capture_bytes(&solo_at_1x),
        "the rebuilt group still costs exactly one capture"
    );
}

/// An anchored overlay's own flush starts with an empty material-scope
/// stack: a member inside the overlay never joins the group the window
/// content around the anchor built.
#[test]
fn a_material_in_an_anchored_overlay_does_not_join_the_outer_group() {
    let open = Binding::container(true);
    let mut runtime = {
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            let open = open.clone();
            AnyView::new(zstack((
                Color::srgb(230, 38, 38),
                vstack((
                    member(Material::Regular),
                    button("host").action(|| {}).anchored_overlay(
                        AnchoredOverlay::new(
                            &open,
                            AnyView::new(().size(60.0, 20.0).background(Material::Regular)),
                        )
                        .edge(AnchorEdge::Bottom),
                    ),
                ))
                .material_group(),
            )))
        });
        HeadlessRuntime::new_for_tests(
            pumped_test_environment(),
            builder,
            WIDTH,
            HEIGHT,
            MinimalTestTheme::default(),
        )
        .with_scale_factor(DISPLAY_SCALE)
    };
    pump(&mut runtime);

    let layers = material_layers(&runtime);
    assert_eq!(
        layers.len(),
        2,
        "the window member and the overlay member both presented"
    );
    let (window_member, overlay_member): (Vec<cherenkov::LayerId>, Vec<cherenkov::LayerId>) =
        layers
            .iter()
            .partition(|&layer| member_scope(&runtime, *layer).is_some());
    assert_eq!(window_member.len(), 1, "the window member keeps its scope");
    assert_eq!(
        overlay_member.len(),
        1,
        "the overlay member flushed with no enclosing group scope"
    );
    let mounts = mounts(&runtime);
    assert_ne!(
        mounts.backdrop_group_id(window_member[0]),
        mounts.backdrop_group_id(overlay_member[0]),
        "the overlay's member is a group of its own"
    );
    assert_eq!(mounts.backdrop_group_count(), 2);
}

/// A grouped `hstack` row measures exactly like the same row ungrouped —
/// the wrapper claims no size of its own — and inside a `VStack::for_each`
/// lazy stack the rows still build, flush under their own group scopes and
/// render: the lazy stack's item measurement looks through the wrapper
/// instead of panicking on an unsupported view type.
#[test]
fn a_grouped_row_in_a_lazy_stack_measures_and_renders() {
    // The item measurement the lazy stack runs before building its rows:
    // `measure_view_dimensions_with_proposal` on a grouped row must answer
    // the `hstack` the group wraps, not the wrapper.
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let mut state = HydroState::new(SessionTextEngine::system(FontFamilyResolution::Strict));
    let proposal = ProposalSize::new(Some(crate::num_cast::u32_as_f32(WIDTH)), None);
    let grouped = normalize_layout_view(AnyView::new(row().material_group()), &env);
    let plain = normalize_layout_view(AnyView::new(row()), &env);
    state.measurement.begin_frame();
    let grouped_size =
        measure_view_dimensions_with_proposal(&grouped, proposal, &mut state, &env, &theme).size;
    state.measurement.begin_frame();
    let plain_size =
        measure_view_dimensions_with_proposal(&plain, proposal, &mut state, &env, &theme).size;
    assert_eq!(
        grouped_size, plain_size,
        "a grouped row measures exactly like the same row ungrouped"
    );

    let mut runtime = rendered(|| {
        AnyView::new(scroll(VStack::for_each(
            (0..40).map(SelfId::new).collect::<Vec<_>>(),
            |_| AnyView::new(row().material_group()),
        )))
    });
    pump(&mut runtime);
    let layers = material_layers(&runtime);
    assert!(!layers.is_empty(), "the lazy stack built its rows");
    assert!(
        layers
            .iter()
            .all(|layer| member_scope(&runtime, *layer).is_some()),
        "every row flushed under its own group's scope"
    );
    // Each row's `.material_group()` is its own modifier instance, so each
    // row is a scope of its own: the table holds one group per row.
    assert_eq!(mounts(&runtime).backdrop_group_count(), layers.len());
}

/// A `.material_group()` around a lazy stack: every row that flushes joins
/// the outer group — including the rows a pan materializes later — so the
/// mount table holds one group for all of them.
#[test]
fn a_group_wrapping_a_lazy_stack_groups_every_flushed_row() {
    let mut runtime = rendered(|| {
        AnyView::new(
            scroll(VStack::for_each(
                (0..40).map(SelfId::new).collect::<Vec<_>>(),
                |_| member(Material::Regular),
            ))
            .material_group(),
        )
    });
    pump(&mut runtime);

    let layers = material_layers(&runtime);
    assert!(!layers.is_empty(), "the lazy stack flushed its first rows");
    assert!(
        layers.iter().all(|layer| {
            member_scope(&runtime, *layer).is_some()
                && member_scope(&runtime, *layer) == member_scope(&runtime, layers[0])
        }),
        "every row flushed under the outer group's scope"
    );
    let group = mounts(&runtime).backdrop_group_id(layers[0]);
    assert!(group.is_some());
    assert!(
        layers
            .iter()
            .all(|layer| mounts(&runtime).backdrop_group_id(*layer) == group),
        "every flushed row shares the one group"
    );
    assert_eq!(mounts(&runtime).backdrop_group_count(), 1);

    // Pan: the rows the stack materializes now flush under the same outer
    // scope and join the same group rather than keying groups of their own.
    runtime.push_input_event(InputEvent::Scroll {
        x: crate::num_cast::u32_as_f32(WIDTH) / 2.0,
        y: crate::num_cast::u32_as_f32(HEIGHT) / 2.0,
        dx: 0.0,
        dy: -400.0,
        is_line_delta: false,
    });
    pump(&mut runtime);

    let layers = material_layers(&runtime);
    assert!(!layers.is_empty(), "the panned stack flushed rows");
    let mounts = mounts(&runtime);
    assert!(
        layers.iter().all(|layer| {
            member_scope(&runtime, *layer).is_some()
                && member_scope(&runtime, *layer) == member_scope(&runtime, layers[0])
        }),
        "rows entering after the pan flushed under the outer scope"
    );
    assert!(
        layers
            .iter()
            .all(|layer| mounts.backdrop_group_id(*layer) == group),
        "rows entering after the pan join the same group"
    );
    assert_eq!(mounts.backdrop_group_count(), 1);
}

/// `view_renders_nothing` looks through `.material_group()`: a grouped
/// empty view claims no stack slot, so the stack measures exactly as if
/// the member were absent.
#[test]
fn a_grouped_empty_view_claims_no_stack_slot() {
    assert!(
        view_renders_nothing(&AnyView::new(().material_group())),
        "a grouped `()` answers renders-nothing like a bare `()`"
    );
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let mut state = HydroState::new(SessionTextEngine::system(FontFamilyResolution::Strict));
    let proposal = ProposalSize::new(Some(200.0), None);
    let grouped = normalize_layout_view(
        AnyView::new(vstack((text("a"), ().material_group(), text("b")))),
        &env,
    );
    let plain = normalize_layout_view(AnyView::new(vstack((text("a"), text("b")))), &env);
    state.measurement.begin_frame();
    let grouped_size =
        measure_view_dimensions_with_proposal(&grouped, proposal, &mut state, &env, &theme).size;
    state.measurement.begin_frame();
    let plain_size =
        measure_view_dimensions_with_proposal(&plain, proposal, &mut state, &env, &theme).size;
    assert_eq!(
        grouped_size, plain_size,
        "a grouped `()` claims no stack slot"
    );
}

/// A bound colour scheme is a live input to the material's flush — the
/// `read_signal` on `current_color_scheme` subscribes the frame — so
/// flipping it re-keys the member into a new group at the next pump: the
/// member's group carries the new scheme's tone and the emptied group
/// releases.
#[test]
fn an_appearance_flip_moves_the_member_to_a_new_group() {
    let scheme = Binding::container(ColorScheme::Light);
    let member_scheme = scheme.clone();
    let mut runtime = {
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(zstack((
                Color::srgb(230, 38, 38),
                member(Material::Regular)
                    .with(Store::<ColorScheme, Computed<ColorScheme>>::new(
                        member_scheme.computed(),
                    ))
                    .material_group(),
            )))
        });
        HeadlessRuntime::new_for_tests(
            pumped_test_environment(),
            builder,
            WIDTH,
            HEIGHT,
            MinimalTestTheme::default(),
        )
        .with_scale_factor(DISPLAY_SCALE)
    };
    pump(&mut runtime);
    let key = material_layers(&runtime)[0];
    assert_eq!(
        mounts(&runtime).backdrop_scheme(key),
        Some(ColorScheme::Light)
    );
    let light_tone = mounts(&runtime).backdrop_tone(key);
    assert_eq!(mounts(&runtime).backdrop_group_count(), 1);

    // The flip is the only change: nothing else requests a frame, so the
    // member's new key — and the old group's release — can only come from
    // the scheme subscription waking the flush.
    scheme.set(ColorScheme::Dark);
    pump(&mut runtime);

    assert_eq!(
        mounts(&runtime).backdrop_scheme(key),
        Some(ColorScheme::Dark),
        "the member re-keyed under the flipped scheme"
    );
    assert_ne!(
        mounts(&runtime).backdrop_tone(key),
        light_tone,
        "the new group's chain carries the dark tone"
    );
    assert_eq!(
        mounts(&runtime).backdrop_group_count(),
        1,
        "the emptied Light group released at the same commit"
    );
}

/// `.material_group()` is a pure scope marker, not an environment wrapper:
/// a handler's captured env resolves through it, so
/// `view.with(state).material_group().on_tap(..)` hands the handler the
/// installed `state`.
#[test]
fn a_handler_through_the_group_wrapper_captures_the_env() {
    let tapped: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let fired = Rc::clone(&tapped);
    let mut runtime = rendered(move || {
        let fired = fired.clone();
        AnyView::new(
            Color::srgb(63, 63, 70)
                .width(20.0)
                .with(42_u32)
                .material_group()
                .on_tap(move |value: Use<u32>| fired.borrow_mut().push(value.0)),
        )
    });
    runtime.push_input_event(InputEvent::PointerDown {
        id: 1,
        kind: PointerKind::Mouse,
        x: 80.0,
        y: 60.0,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump_at(false, Instant::now());
    runtime.push_input_event(InputEvent::PointerUp {
        id: 1,
        kind: PointerKind::Mouse,
        x: 80.0,
        y: 60.0,
        button: PointerButton::Primary,
    });
    pump(&mut runtime);
    assert_eq!(
        tapped.borrow().as_slice(),
        &[42],
        "the handler extracted the env value through the group wrapper"
    );
}

/// `.material_group()` is transparent to a `List` row's label hoisting:
/// `content.a11y_label(..).material_group()` in a row claims the label
/// exactly as the unwrapped content does.
#[cfg(feature = "accessibility")]
#[test]
fn a_grouped_list_row_hoists_its_accessibility_label() {
    fn roles_and_labels(grouped: bool) -> Vec<(Role, Option<String>)> {
        let mut runtime = {
            let builder = AnyViewBuilder::<AnyView>::new(move || {
                let rows = Binding::container(vec![SelfId::new(1_u64)]);
                AnyView::new(List::for_each(SignalCollection::new(rows), move |_| {
                    let content = text("inside").a11y_label("row label");
                    ListItem::new(if grouped {
                        AnyView::new(content.material_group())
                    } else {
                        AnyView::new(content)
                    })
                }))
            });
            HeadlessRuntime::new_for_tests(
                test_environment(),
                builder,
                WIDTH,
                HEIGHT,
                MinimalTestTheme::default(),
            )
        };
        pump(&mut runtime);
        let mut found: Vec<_> = runtime
            .accessibility_tree()
            .into_iter()
            .flat_map(|update| update.nodes)
            .map(|(_, node)| (node.role(), node.label().map(str::to_owned)))
            .collect();
        found.sort_unstable();
        found
    }
    let plain = roles_and_labels(false);
    assert!(
        plain.contains(&(Role::ListItem, Some(String::from("row label")))),
        "baseline: the row claims the hoisted label, got {plain:?}"
    );
    assert_eq!(
        plain,
        roles_and_labels(true),
        "the group wrapper changes nothing about the row's tree"
    );
}
