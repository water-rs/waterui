//! `.material_group()` (water-rs/waterui#1999): the `IgnorableMetadata`
//! wrapper opens a material scope the flush keeps on a stack, and the mount
//! table's group key `(scope, within-window level, resolved colour scheme,
//! install canvas)` makes the members under one scope share one
//! `cherenkov::BackdropGroup` — one capture, one chain. A member outside
//! every group is a group of its own; members of different levels, schemes,
//! canvases, or scopes never share.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

#[cfg(feature = "accessibility")]
use accesskit::Role;
use nami::collection::List as Membership;
#[cfg(feature = "accessibility")]
use nami::collection::SignalCollection;
use nami::{Binding, Computed, SignalExt as _};
use waterui::FilterViewExt as _;
use waterui::ViewExt as _;
use waterui::animation::Animation;
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
use waterui_layout::collection_transition::collection_transition;
use waterui_layout::scroll;
use waterui_layout::stack::{VStack, hstack, vstack, zstack};

use super::{
    MinimalTestTheme, capture_bytes, inner_layers, material_layers, mounted_anchor_parent_children,
    mounts, pumped_test_environment, test_environment, window_mount,
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

/// An empty `MEMBER`-sized view — keeps a solo-member reference's stack
/// layout identical to the pair's.
fn spacer() -> AnyView {
    AnyView::new(().size(MEMBER.0, MEMBER.1))
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

/// Pumps until the renderer settles, advancing the frame clock past any
/// exit-transition's duration so retained scopes actually retire.
fn pump_until_settled(runtime: &mut HeadlessRuntime) {
    let start = Instant::now();
    for frame in 0..240 {
        let _ = runtime.pump_at(false, start + Duration::from_secs(frame));
        if runtime.is_settled() {
            break;
        }
    }
}

fn assert_anchor_children_match(runtime: &mut HeadlessRuntime, anchor: cherenkov::LayerId) -> bool {
    let members = material_layers(runtime);
    assert_eq!(members.len(), 1, "the fixture has one material member");
    let member = members[0];
    let (scope, canvas, _) = mounts(runtime)
        .anchor_registrations()
        .into_iter()
        .find(|(_, _, layer)| *layer == anchor)
        .expect("the anchor has a registration");
    assert_eq!(canvas, None, "the fixture has no installation canvas");
    let (_, engine_children, direct) =
        mounted_anchor_parent_children(runtime, scope, canvas, anchor)
            .expect("the anchor has committed children");
    assert_eq!(
        engine_children.len(),
        2,
        "the anchor parent has exactly one anchor and one member"
    );
    assert_eq!(
        engine_children.iter().filter(|&&id| id == anchor).count(),
        1,
        "the engine child list has one anchor"
    );
    assert_eq!(
        engine_children.iter().filter(|&&id| id == member).count(),
        1,
        "the engine child list has one member"
    );

    runtime.renderer_mut().commit_mirror();
    let (mirror_anchor, mirror_member, mirror_children) = {
        let mirror = runtime.renderer().mirror();
        let mirror_anchor = mirror
            .anchor_registrations()
            .into_iter()
            .find(|&(mirror_scope, mirror_canvas, _)| {
                mirror_scope == scope && mirror_canvas == canvas
            })
            .map(|(_, _, layer)| layer)
            .expect("the mirror has the matching anchor registration");
        let mirror_members = mirror.backdrops();
        assert_eq!(
            mirror_members.len(),
            1,
            "the mirror fixture has one material member"
        );
        let mirror_member = mirror_members[0].0;
        let mirror_parent = mirror
            .parent(mirror_anchor)
            .unwrap_or_else(|| panic!("mirror anchor {mirror_anchor:?} has no parent"));
        (mirror_anchor, mirror_member, mirror.children(mirror_parent))
    };
    assert_ne!(mirror_anchor, mirror_member);
    assert_eq!(
        mirror_children.len(),
        2,
        "the mirror child list has exactly one anchor and one member"
    );
    let mapped = mirror_children
        .iter()
        .map(|&id| {
            if id == mirror_anchor {
                anchor
            } else if id == mirror_member {
                member
            } else {
                panic!("unmapped mirror child {id:?}");
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(mapped, engine_children);
    direct
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
        "two members in one group cost exactly one capture — the \
         anchored group reads the semantic target at its anchor, no \
         shared copy"
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
        "the rebuilt group costs exactly its 1x capture"
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
    assert!(
        runtime.pump_at(true, Instant::now()).snapshot.is_some(),
        "the lazy stack's grouped rows render a snapshot"
    );
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

/// A full-window `Thick` member — sized to the window, not `MEMBER` —
/// under the same helpers `member` uses.
#[expect(
    clippy::cast_precision_loss,
    reason = "the window size is exact in f32"
)]
fn thick_full() -> AnyView {
    AnyView::new(
        ().size(WIDTH as f32, HEIGHT as f32)
            .background(Material::Thick),
    )
}

/// The window's pixels after one rendered frame at `DISPLAY_SCALE`.
fn snap(view: impl Fn() -> AnyView + 'static) -> crate::HeadlessSnapshot {
    let mut runtime = rendered(view);
    // Filtered nodes set up their capture asynchronously — the first pump
    // mounts them, later frames run the pipeline. Render a handful of real
    // frames so a filtered subtree has painted before the snapshot.
    let start = Instant::now();
    let mut snapshot = None;
    for frame in 0..8 {
        let pumped = runtime.pump_at(true, start + Duration::from_millis(frame * 16));
        if pumped.snapshot.is_some() {
            snapshot = pumped.snapshot;
        }
    }
    snapshot.expect("a snapshot")
}

fn px(snap: &crate::HeadlessSnapshot, x: usize, y: usize) -> [u8; 4] {
    let i = (y * snap.width as usize + x) * 4;
    snap.rgba8[i..i + 4].try_into().unwrap()
}

/// A `.material_group()`'s scope gets a plain layer at the scope's paint
/// position and every backdrop group the scope keys anchors at it
/// (water-rs/waterui#2097): a `Regular` member and then a `Thick` member
/// overlapping it capture beneath the anchor — so the `Thick` capture
/// holds only what painted before the scope, never the `Regular` member.
#[test]
fn a_scopes_groups_capture_beneath_its_anchor() {
    let scope_view = |grouped: bool| {
        move || {
            let members = zstack((member(Material::Regular), thick_full()));
            AnyView::new(zstack((
                Color::srgb(230, 38, 38),
                if grouped {
                    AnyView::new(members.material_group())
                } else {
                    AnyView::new(members)
                },
            )))
        }
    };
    let (grouped_runtime, grouped) = {
        let mut runtime = rendered(scope_view(true));
        let snap = runtime.pump_snapshot().snapshot.expect("a snapshot");
        (runtime, snap)
    };
    // The solo `Thick` member's own first-member capture holds only the
    // red backdrop — exactly what the anchored copy holds too.
    let solo = snap(|| AnyView::new(zstack((Color::srgb(230, 38, 38), thick_full()))));
    let layers = material_layers(&grouped_runtime);
    assert_eq!(layers.len(), 2);
    let scope =
        member_scope(&grouped_runtime, layers[0]).expect("the member flushed under a scope");
    assert_eq!(member_scope(&grouped_runtime, layers[1]), Some(scope));
    // The anchor written into each group's spec: the plain layer the
    // scope's anchor item mounts, and the same one for every member.
    let anchor = mounts(&grouped_runtime).backdrop_anchor(layers[0]);
    assert!(anchor.is_some(), "the scope's groups anchor at its layer");
    for layer in &layers {
        assert_eq!(
            mounts(&grouped_runtime).backdrop_anchor(*layer),
            anchor,
            "every group the scope keys anchors at the scope's anchor layer"
        );
    }
    // Outside the `Regular` member's bounds — the window corners and just
    // above its top edge — the `Thick` member's output equals the solo
    // member's: the shared capture holds the bare red backdrop. A capture
    // taken at the member's own paint position would hold the `Regular`
    // panel there and the blur would carry it past the panel's edge.
    for point in [(10, 10), (310, 10), (10, 230), (310, 230), (160, 55)] {
        assert_eq!(
            px(&grouped, point.0, point.1),
            px(&solo, point.0, point.1),
            "pixel {point:?}: the `Thick` output differs from the solo capture"
        );
    }
    // Inside the `Regular` member's bounds the `Thick` member covers it
    // fully, so its composite reads exactly what it sampled: the bare red
    // backdrop the anchor froze — the solo capture, again. The
    // ungrouped comparison below proves the `Regular` member did paint:
    // it is what makes the ungrouped capture differ.
    assert_eq!(
        px(&grouped, 160, 120),
        px(&solo, 160, 120),
        "inside the member's bounds the anchored `Thick` sample matches \
         the solo capture — the `Regular` member never entered it"
    );
    // Ungrouped, the `Thick` member's first-member capture does hold the
    // `Regular` member: the overlap and the panel's blur reach differ.
    let ungrouped = snap(scope_view(false));
    for point in [(160, 120), (160, 55)] {
        assert_ne!(
            px(&grouped, point.0, point.1),
            px(&ungrouped, point.0, point.1),
            "pixel {point:?}: the anchored capture holds the `Regular` member"
        );
    }
}

/// The scope's anchor mount releases with the scope: when the
/// `.material_group()` leaves the frame its anchor layer is pruned from
/// the mount table.
#[test]
fn a_scopes_anchor_releases_with_the_scope() {
    let shown = Binding::container(true);
    let mut runtime = {
        let shown = shown.clone();
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(zstack((
                Color::srgb(230, 38, 38),
                when(shown.clone(), || member(Material::Regular).material_group()),
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
        let start = Instant::now();
        for step in 0..=80 {
            let _ = runtime.pump_at(false, start + Duration::from_millis(step * 16));
        }
        runtime
    };
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 1);
    assert!(
        member_scope(&runtime, layers[0]).is_some(),
        "the member flushed under a scope"
    );
    assert!(
        mounts(&runtime).backdrop_anchor(layers[0]).is_some(),
        "the member's group anchors at the scope's anchor layer"
    );

    shown.set(false);
    pump_until_settled(&mut runtime);
    assert_eq!(material_layers(&runtime), Vec::<cherenkov::LayerId>::new());
    assert_eq!(mounts(&runtime).backdrop_group_count(), 0);
    assert_eq!(
        mounts(&runtime).anchor_registration_count(),
        0,
        "the scope's anchor registration released with it"
    );
    // The exited scope's anchor layer is gone from the engine's child
    // list, not merely unregistered: the engine's committed children are
    // structurally the mirror's own commit — a stale retained anchor
    // layer would show as an extra engine child skewing `reconcile`'s
    // insert indices. The two mounts own separate layer-id spaces, so
    // the comparison is structural (count and position), not per-id.
    runtime.renderer_mut().commit_mirror();
    assert_eq!(
        window_mount(&runtime).window_children().len(),
        runtime.renderer().mirror().window_children().len(),
        "the engine's window child list matches the mirror's"
    );
}

/// The group's spec carries the anchor item's layer when the scope's
/// content root is itself a member (water-rs/waterui#2097): the anchor
/// item the `.material_group()` flush pushes is registered before the
/// member commits, so a member that is also the scope's content root is
/// still anchored at the plain layer the item mounts.
#[test]
fn a_member_as_the_scopes_content_root_is_anchored() {
    let runtime = rendered(|| AnyView::new(member(Material::Regular).material_group()));
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 1);
    let anchor = mounts(&runtime).backdrop_anchor(layers[0]);
    assert!(
        anchor.is_some(),
        "the member's group anchors at the scope's anchor layer"
    );
    assert_ne!(
        anchor,
        Some(layers[0]),
        "the anchor is a layer of its own, not the member's frame"
    );
}

/// Pass-through content roots (gesture, env, retain wrappers) do not
/// hide the anchor item: `.material_group()` pushes it into the
/// enclosing program wherever the scope's content hangs, so the members
/// inside anchor at it.
#[test]
fn a_passthrough_content_root_keeps_the_scope_anchored() {
    let runtime = rendered(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            vstack((member(Material::Regular), thick_full()))
                .on_tap(|| {})
                .material_group(),
        )))
    });
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 2);
    let anchor = mounts(&runtime).backdrop_anchor(layers[0]);
    assert!(
        anchor.is_some(),
        "the members' group anchors at the scope's anchor layer"
    );
    for layer in &layers {
        assert_eq!(mounts(&runtime).backdrop_anchor(*layer), anchor);
    }
}

/// A scope inside a filtered view mounts its anchor item on the filter's
/// canvas, and the members' groups on that canvas anchor at it.
#[test]
fn a_scope_inside_a_filtered_view_anchors_on_its_canvas() {
    let runtime = rendered(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            zstack((member(Material::Regular), thick_full()))
                .material_group()
                .blur(2.0f32),
        )))
    });
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 2);
    let anchor = mounts(&runtime).backdrop_anchor(layers[0]);
    assert!(
        anchor.is_some(),
        "the members' group anchors at the scope's anchor layer"
    );
    for layer in &layers {
        assert_eq!(mounts(&runtime).backdrop_anchor(*layer), anchor);
    }
}

/// A partial re-record that re-records the anchor's scope node — here a
/// `when` child inside the `.material_group()` toggled by a binding —
/// leaves the anchor item and the members' spec anchor in place. (This
/// deliberately re-records the scope node itself rather than a
/// descendant that skips the scope flush, which is #2268's shape.)
#[test]
fn a_partial_re_record_keeps_the_scope_anchored() {
    let shown = Binding::container(true);
    let mut runtime = {
        let shown = shown.clone();
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(zstack((
                Color::srgb(230, 38, 38),
                vstack((member(Material::Regular), when(shown.clone(), thick_full)))
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
    let anchor = mounts(&runtime).backdrop_anchor(layers[0]);
    assert!(anchor.is_some());

    shown.set(false);
    pump(&mut runtime);
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 1);
    assert_eq!(
        mounts(&runtime).backdrop_anchor(layers[0]),
        anchor,
        "the surviving member's group still anchors at the same layer"
    );
}

/// A `.material_group()` row leaving a `collection_transition` stack moves
/// between two mount lists mid-flight: while its exit transition runs, the
/// row's scope cell flushes inside the departing entry's `Item::Scope` list
/// instead of the collection's item list. The scope keeps exactly one anchor
/// registration, and the final anchor-parent snapshot compares the complete
/// ordered child list against the mirror (water-rs/waterui#2097, MEDIUM-1).
#[test]
fn a_scope_moving_between_lists_keeps_one_owner() {
    let rows: Membership<SelfId<u64>> = Membership::from(vec![SelfId::new(1)]);
    let mut runtime = {
        let rows = rows.clone();
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            let rows = rows.clone();
            let collection = VStack::for_each(rows, move |_| {
                AnyView::new(member(Material::Regular).material_group())
            });
            AnyView::new(collection_transition(
                collection,
                Animation::linear(Duration::from_millis(1_000)),
            ))
        });
        let mut runtime = HeadlessRuntime::new_for_tests(
            pumped_test_environment(),
            builder,
            WIDTH,
            HEIGHT,
            MinimalTestTheme::default(),
        )
        .with_scale_factor(DISPLAY_SCALE);
        let start = Instant::now();
        for step in 0..=80 {
            let _ = runtime.pump_at(false, start + Duration::from_millis(step * 16));
        }
        runtime
    };
    assert_eq!(material_layers(&runtime).len(), 1);
    assert_eq!(mounts(&runtime).anchor_registration_count(), 1);
    let old_member = material_layers(&runtime)[0];
    let old_anchor = mounts(&runtime)
        .backdrop_anchor(old_member)
        .expect("the stable row has an anchor");

    // Removing the row moves its scope cell from the collection's item
    // list into the departing entry's `Item::Scope` list — pump one frame
    // into the exit transition, before the entry retires. A synthetic
    // clock keeps the assert mid-transition: `pump` runs on wall time,
    // which can run the 1 s animation out under suite load.
    let _ = rows.remove(0);
    let start = Instant::now();
    for step in 0..64 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(step * 15));
    }
    let registrations = mounts(&runtime).anchor_registrations();
    assert_eq!(
        registrations.len(),
        1,
        "one live registration while the row exits inside its scope item"
    );
    let member_layer = material_layers(&runtime)[0];
    assert_eq!(member_layer, old_member);
    let new_anchor = mounts(&runtime)
        .backdrop_anchor(member_layer)
        .expect("the departing member still has an anchor");
    assert_ne!(new_anchor, old_anchor);
    assert_eq!(
        Some(new_anchor),
        Some(registrations[0].2),
        "the departing member still anchors at its own scope's layer"
    );

    // Once the transition retires the entry, nothing stands: no member,
    // no group, no registration.
    assert_anchor_children_match(&mut runtime, new_anchor);
    pump_until_settled(&mut runtime);
    assert_eq!(material_layers(&runtime).len(), 0);
    assert_eq!(mounts(&runtime).backdrop_group_count(), 0);
    assert_eq!(mounts(&runtime).anchor_registration_count(), 0);
}

#[test]
fn a_scope_enter_completion_keeps_one_direct_owner() {
    let rows: Membership<SelfId<u64>> = Membership::from(vec![]);
    let builder = {
        let rows = rows.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let rows = rows.clone();
            let collection = VStack::for_each(rows, move |_| {
                AnyView::new(member(Material::Regular).material_group())
            });
            AnyView::new(collection_transition(
                collection,
                Animation::linear(Duration::from_millis(1_000)),
            ))
        })
    };
    let mut runtime = HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        WIDTH,
        HEIGHT,
        MinimalTestTheme::default(),
    )
    .with_scale_factor(DISPLAY_SCALE);
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    rows.push(SelfId::new(1));
    for step in 1..=16 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(step * 16));
    }
    let mid_member = material_layers(&runtime);
    assert_eq!(mid_member.len(), 1);
    assert_eq!(mounts(&runtime).anchor_registration_count(), 1);
    let mid_member = mid_member[0];
    let mid_anchor = mounts(&runtime)
        .backdrop_anchor(mid_member)
        .expect("the entering member has an anchor");
    for step in 17..=80 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(step * 16));
    }
    let final_members = material_layers(&runtime);
    assert_eq!(final_members.len(), 1);
    assert_eq!(final_members[0], mid_member);
    assert_eq!(mounts(&runtime).anchor_registration_count(), 1);
    let final_anchor = mounts(&runtime)
        .backdrop_anchor(final_members[0])
        .expect("the settled member has an anchor");
    assert_ne!(final_anchor, mid_anchor);
    let registrations = mounts(&runtime).anchor_registrations();
    assert_eq!(registrations.len(), 1);
    assert_eq!(registrations[0].2, final_anchor);
    assert!(
        assert_anchor_children_match(&mut runtime, final_anchor),
        "the settled anchor is in the direct frame list"
    );
}

#[test]
fn a_scope_reinserted_during_exit_keeps_one_direct_owner() {
    let rows: Membership<SelfId<u64>> = Membership::from(vec![SelfId::new(1)]);
    let builder = {
        let rows = rows.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let rows = rows.clone();
            let collection = VStack::for_each(rows, move |_| {
                AnyView::new(member(Material::Regular).material_group())
            });
            AnyView::new(collection_transition(
                collection,
                Animation::linear(Duration::from_millis(1_000)),
            ))
        })
    };
    let mut runtime = HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        WIDTH,
        HEIGHT,
        MinimalTestTheme::default(),
    )
    .with_scale_factor(DISPLAY_SCALE);
    let start = Instant::now();
    for step in 0..=80 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(step * 16));
    }
    assert_eq!(material_layers(&runtime).len(), 1);
    assert_eq!(mounts(&runtime).anchor_registration_count(), 1);

    let _ = rows.remove(0);
    let _ = runtime.pump_at(false, start + Duration::from_millis(1_300));
    assert_eq!(mounts(&runtime).anchor_registration_count(), 1);

    rows.push(SelfId::new(1));
    for step in 90..=160 {
        let _ = runtime.pump_at(false, start + Duration::from_millis(step * 16));
    }
    let members = material_layers(&runtime);
    assert_eq!(members.len(), 1);
    assert_eq!(mounts(&runtime).anchor_registration_count(), 1);
    let anchor = mounts(&runtime)
        .backdrop_anchor(members[0])
        .expect("the reinserted row has an anchor");
    let registrations = mounts(&runtime).anchor_registrations();
    assert_eq!(registrations.len(), 1);
    assert_eq!(registrations[0].2, anchor);
    assert!(
        assert_anchor_children_match(&mut runtime, anchor),
        "the settled reinserted anchor is in the direct frame list"
    );
}

/// A scope unmounted and re-mounted gets a fresh anchor layer: the group
/// re-keys on the layer the item mounts now, never on a stale one.
#[test]
fn a_scope_returning_after_unmount_reanchors() {
    let shown = Binding::container(true);
    let mut runtime = {
        let shown = shown.clone();
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(zstack((
                Color::srgb(230, 38, 38),
                when(shown.clone(), || member(Material::Regular).material_group()),
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
    let first = mounts(&runtime)
        .backdrop_anchor(layers[0])
        .expect("anchored");

    shown.set(false);
    pump_until_settled(&mut runtime);
    shown.set(true);
    pump(&mut runtime);
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 1);
    let second = mounts(&runtime).backdrop_anchor(layers[0]);
    assert!(
        second.is_some(),
        "the remounted scope's group anchors at its new anchor layer"
    );
    assert_ne!(
        second,
        Some(first),
        "the group re-keyed onto the remounted scope's fresh anchor layer"
    );
    assert_eq!(mounts(&runtime).anchor_registration_count(), 1);
}

/// Members of a scope that mount inside a filtered node's canvas anchor
/// at the item the filtered program pushes at its start: the capture
/// reads that canvas as it stands at the anchor's paint position —
/// empty — so no member's material shows another member. Each member's
/// pixels are byte-equal to the same member rendered solo in the same
/// filtered, grouped shape (water-rs/waterui#2097).
#[test]
fn members_inside_a_filtered_view_capture_beneath_the_filter() {
    let grouped = snap(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            vstack((member(Material::Regular), member(Material::Thick)))
                .material_group()
                .blur(2.0f32),
        )))
    });
    let solo_regular = snap(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            vstack((member(Material::Regular), spacer()))
                .material_group()
                .blur(2.0f32),
        )))
    });
    let solo_thick = snap(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            vstack((spacer(), member(Material::Thick)))
                .material_group()
                .blur(2.0f32),
        )))
    });
    // The `Regular` member occupies y∈[0,60]pt and the `Thick`
    // y∈[60,120]pt of the window (device pixels double); these points
    // sit inside each member's own region, far from the blur's bleed
    // over the other's edge.
    for point in [(160, 60), (160, 100)] {
        assert_eq!(
            px(&grouped, point.0, point.1),
            px(&solo_regular, point.0, point.1),
            "pixel {point:?}: the `Regular` member renders as though alone"
        );
    }
    for point in [(160, 140), (160, 200)] {
        assert_eq!(
            px(&grouped, point.0, point.1),
            px(&solo_thick, point.0, point.1),
            "pixel {point:?}: the `Thick` member renders as though alone"
        );
    }
}

/// The decided shape: a two-level pair inside `.blur()` with the scope
/// enclosing the filtered node. The filtered program pushes the scope's
/// anchor item at its start, so the members' spec anchor is that
/// F-start anchor — and its capture reads the filtered canvas as it
/// stands there: empty. No member shows another, verified in pixels
/// against solo-member references (water-rs/waterui#2097).
#[test]
fn a_scope_enclosing_a_filtered_view_anchors_at_the_filter_start() {
    let runtime = rendered(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            vstack((member(Material::Regular), member(Material::Thick)))
                .blur(2.0f32)
                .material_group(),
        )))
    });
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 2);
    let anchor = mounts(&runtime)
        .backdrop_anchor(layers[0])
        .expect("the members' group anchors");
    // The anchor item the filtered program pushes at its start registers
    // under the filtered canvas — the one registration naming a canvas.
    let f_start = mounts(&runtime)
        .anchor_registrations()
        .into_iter()
        .find(|(_, canvas, _)| canvas.is_some())
        .map(|(_, _, layer)| layer);
    assert_eq!(
        Some(anchor),
        f_start,
        "the spec anchor is the F-start anchor"
    );
    for layer in &layers {
        assert_eq!(mounts(&runtime).backdrop_anchor(*layer), Some(anchor));
    }

    let grouped = snap(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            vstack((member(Material::Regular), member(Material::Thick)))
                .blur(2.0f32)
                .material_group(),
        )))
    });
    let solo_regular = snap(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            vstack((member(Material::Regular), spacer()))
                .blur(2.0f32)
                .material_group(),
        )))
    });
    let solo_thick = snap(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            vstack((spacer(), member(Material::Thick)))
                .blur(2.0f32)
                .material_group(),
        )))
    });
    for point in [(160, 60), (160, 100)] {
        assert_eq!(
            px(&grouped, point.0, point.1),
            px(&solo_regular, point.0, point.1),
            "pixel {point:?}: the `Regular` member renders as though alone"
        );
    }
    for point in [(160, 140), (160, 200)] {
        assert_eq!(
            px(&grouped, point.0, point.1),
            px(&solo_thick, point.0, point.1),
            "pixel {point:?}: the `Thick` member renders as though alone"
        );
    }
}

/// A `.material_group()` inside a scroll view's content mounts its
/// anchor layer under the scrolled inner layer — the inner list owns
/// its own anchor table, so the outer list's lowering never retires the
/// anchor the inner pass just created (water-rs/waterui#2097).
#[test]
fn a_scope_inside_a_scroll_view_anchors_under_the_inner_layer() {
    let mut runtime = rendered(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            scroll(member(Material::Regular).material_group()),
        )))
    });
    assert!(
        runtime.pump_at(true, Instant::now()).snapshot.is_some(),
        "the scrolled member's frame renders a snapshot"
    );
    let layers = material_layers(&runtime);
    assert_eq!(layers.len(), 1);
    let anchor = mounts(&runtime)
        .backdrop_anchor(layers[0])
        .expect("the scrolled member's group anchors");
    // The spec anchor is a child of the inner layer: it and the member's
    // frame are siblings under the same parent.
    let (scope, canvas, _) = mounts(&runtime)
        .anchor_registrations()
        .into_iter()
        .find(|(_, _, layer)| *layer == anchor)
        .expect("the scrolled anchor has a registration");
    assert_eq!(canvas, None);
    let (parent, _, _) = mounted_anchor_parent_children(&runtime, scope, canvas, anchor)
        .expect("the scrolled anchor has a committed parent");
    let inner = inner_layers(&runtime);
    assert_eq!(inner.len(), 1);
    assert_eq!(parent, inner[0]);

    // Pixels: the scrolled member paints its material over the backdrop —
    // the member's pixel differs from the same scene with no member.
    let member_shot = snap(|| {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            scroll(member(Material::Regular).material_group()),
        )))
    });
    let empty_shot = snap(|| AnyView::new(zstack((Color::srgb(230, 38, 38), scroll(spacer())))));
    assert_ne!(
        px(&member_shot, 80, 60),
        px(&empty_shot, 80, 60),
        "the scrolled member paints its material over the backdrop"
    );
}
