//! The `map` leaf: `Native<MapConfig>` rendered through the kit's `MKMapView`
//! wrapper.
//!
//! Mirrors `WuiMapComponent`: the `camera` `Binding<Region>` pushes
//! `setRegion` (animated when the watcher metadata carries an `Animation`) and
//! receives settled user pan/zoom gestures back via `on_region_change`;
//! `annotations` reconciles by [`Id`] — rendered pins are reused in place by
//! identity, removed when absent, added when new — optional `selection` syncs
//! two-way with `MapKit` annotation selection, an application-supplied
//! `user_location` replaces `showsUserLocation` tracking with the map's own
//! marker and accuracy circle, and a `status` `Binding` receives `MapKit`'s
//! load lifecycle. Every update is an imperative kit call inside a watcher —
//! no signal type crosses into `cocoa-ui`.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use std::collections::HashMap;

use cocoa_ui::map::{
    Annotation as KitAnnotation, AnnotationMarker, AnnotationSelectionEvent,
    Coordinate as KitCoordinate, LoadStatus, MapView, Region as KitRegion, Span as KitSpan,
    Style as KitStyle, SuppliedLocation,
};
use nami::Signal;
use nami::watcher::Metadata;
use objc2::rc::Retained;
use waterui_core::Str;
use waterui_core::animation::ChangeAnimation;
use waterui_core::id::Id;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView, ViewDimensions,
};
use waterui_map::{Coordinate, MapConfig, MapStatus, MapStyle, Region, ResolvedAnnotation};

use crate::contract::{NativeLeaf, RenderContext};

/// Angular tolerance in degrees (~0.11 m at the equator) below which a
/// `MapKit`-reported region is treated as matching the bound region so
/// floating-point rounding does not bounce updates back into `setRegion`.
const REGION_EPSILON: f64 = 1e-6;

/// Whether this watcher callback animates: the change names an
/// `Animation`, the same check `metadata.animation != nil` performed.
fn animated(metadata: &Metadata) -> bool {
    ChangeAnimation::of(metadata).animation().is_some()
}

/// A `waterui_map::Coordinate` as the kit's degree-pair.
const fn kit_coordinate(coordinate: &Coordinate) -> KitCoordinate {
    KitCoordinate::new(coordinate.latitude.get(), coordinate.longitude.get())
}

/// A `waterui_map::Region` as the kit's center-and-span region.
const fn kit_region(region: &Region) -> KitRegion {
    KitRegion::new(
        kit_coordinate(&region.center),
        KitSpan::new(region.latitude_delta, region.longitude_delta),
    )
}

/// Converts a `MapKit` region back into a validated `waterui_map::Region`.
fn map_region(region: &KitRegion) -> Option<Region> {
    let center = Coordinate::from_degrees(region.center.latitude, region.center.longitude).ok()?;
    let lat_delta = region.span.latitude_delta;
    let lon_delta = region.span.longitude_delta;
    if !lat_delta.is_finite() || !lon_delta.is_finite() {
        return None;
    }
    Some(Region::new(center, lat_delta.max(0.0), lon_delta.max(0.0)))
}

/// Returns whether two regions agree within [`REGION_EPSILON`] on center and span.
fn regions_close(a: &Region, b: &Region) -> bool {
    (a.center.latitude.get() - b.center.latitude.get()).abs() <= REGION_EPSILON
        && (a.center.longitude.get() - b.center.longitude.get()).abs() <= REGION_EPSILON
        && (a.latitude_delta - b.latitude_delta).abs() <= REGION_EPSILON
        && (a.longitude_delta - b.longitude_delta).abs() <= REGION_EPSILON
}

/// A `waterui_map::ResolvedAnnotation` as the kit's pin value.
fn kit_annotation(annotation: &ResolvedAnnotation) -> KitAnnotation {
    KitAnnotation {
        coordinate: kit_coordinate(&annotation.coordinate()),
        title: annotation.title().as_str().to_owned(),
        subtitle: annotation
            .subtitle()
            .as_ref()
            .map(|text| text.as_str().to_owned()),
    }
}

/// A `MapKit` imagery style for a `MapStyle`.
const fn kit_style(style: MapStyle) -> KitStyle {
    match style {
        MapStyle::Standard => KitStyle::Standard,
        MapStyle::Satellite => KitStyle::Satellite,
        MapStyle::Hybrid => KitStyle::Hybrid,
    }
}

/// A `MapKit` load event as the contract's `MapStatus`.
fn map_status(status: &LoadStatus) -> MapStatus {
    match status {
        LoadStatus::Loading => MapStatus::Loading,
        LoadStatus::Ready => MapStatus::Ready,
        LoadStatus::Failed(reason) => MapStatus::Failed(Str::from(reason.clone())),
    }
}

/// The leaf's live state: the markers currently on the map keyed by erased
/// [`Id`], reconciled against `config.annotations` on every change. `KeepAlive`
/// holds the `Rc`.
#[derive(Debug)]
struct MapState {
    /// Rendered annotation handles in display order, paired with their erased [`Id`].
    markers: Vec<(Id, AnnotationMarker)>,
    /// Currently selected annotation [`Id`], mirrored with `MapKit`.
    selected: Option<Id>,
}

impl MapState {
    fn marker_for(&self, id: Id) -> Option<&AnnotationMarker> {
        self.markers
            .iter()
            .find_map(|(entry_id, marker)| (*entry_id == id).then_some(marker))
    }

    fn id_for_identity(&self, identity: usize) -> Option<Id> {
        self.markers
            .iter()
            .find_map(|(entry_id, marker)| (marker.identity() == identity).then_some(*entry_id))
    }
}

/// Reconciles `existing_markers` against `values` by [`Id`], updating reused
/// markers in place, removing absent ones, and batch-adding new ones while
/// preserving the exact order of `values`.
fn reconcile_markers<M>(
    existing_markers: &mut Vec<(Id, M)>,
    values: &[ResolvedAnnotation],
    mut update: impl FnMut(&M, &KitAnnotation),
    mut remove: impl FnMut(Vec<M>),
    mut add: impl FnMut(&[KitAnnotation]) -> Vec<M>,
) {
    let mut existing: HashMap<Id, M> = existing_markers.drain(..).collect();
    let mut merged: Vec<Option<(Id, M)>> = (0..values.len()).map(|_| None).collect();
    let mut added_entries = Vec::new();
    let mut added_values = Vec::new();

    for (slot, value) in values.iter().enumerate() {
        let id = value.id();
        if let Some(marker) = existing.remove(&id) {
            update(&marker, &kit_annotation(value));
            merged[slot] = Some((id, marker));
        } else {
            added_entries.push((slot, id));
            added_values.push(kit_annotation(value));
        }
    }

    if !existing.is_empty() {
        remove(existing.into_values().collect());
    }

    if !added_values.is_empty() {
        let added_markers = add(&added_values);
        for ((slot, id), marker) in added_entries.into_iter().zip(added_markers) {
            merged[slot] = Some((id, marker));
        }
    }

    *existing_markers = merged.into_iter().flatten().collect();
}

/// Applies `values` onto the map, reconciling rendered markers by [`Id`] so
/// stable pins keep their native object identity and selection across updates.
fn reconcile_annotations(
    view: &MapView,
    state: &mut MapState,
    values: &[ResolvedAnnotation],
    suppressing_selection: &Cell<bool>,
) {
    reconcile_markers(
        &mut state.markers,
        values,
        |marker, annotation| view.update_annotation(marker, annotation),
        |removed| {
            suppressing_selection.set(true);
            view.remove_annotations(&removed);
            suppressing_selection.set(false);
        },
        |added| view.add_annotations(added),
    );

    if let Some(selected_id) = state.selected
        && let Some(marker) = state.marker_for(selected_id)
    {
        suppressing_selection.set(true);
        view.select_annotation(marker, false);
        suppressing_selection.set(false);
    }
}

/// Applies programmatic selection onto `MapKit` while suppressing delegate
/// write-back echo.
fn apply_selection(
    view: &MapView,
    state: &mut MapState,
    next: Option<Id>,
    animated: bool,
    suppressing_selection: &Cell<bool>,
) {
    if state.selected == next {
        return;
    }
    let previous = state.selected;
    state.selected = next;
    suppressing_selection.set(true);
    if let Some(prev_id) = previous
        && let Some(marker) = state.marker_for(prev_id)
    {
        view.deselect_annotation(marker, animated);
    }
    if let Some(next_id) = next
        && let Some(marker) = state.marker_for(next_id)
    {
        view.select_annotation(marker, animated);
    }
    suppressing_selection.set(false);
}

/// The map's layout face: greedy — stretches both axes, falling back to the
/// Swift port's 320x480 when a proposal dimension is missing.
struct MapSubView;

impl core::fmt::Debug for MapSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MapSubView").finish_non_exhaustive()
    }
}

impl SubView for MapSubView {
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(320.0),
            proposal.height.unwrap_or(480.0),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Wires two-way camera synchronization between `config.camera()` and `map`.
fn bind_camera(map: &Retained<MapView>, leaf: &mut NativeLeaf, config: &MapConfig) {
    let camera = config.camera().clone();
    let writing_back = Rc::new(Cell::new(false));

    // The region snapshot applies unanimated at mount; each change
    // animates only when the watcher metadata asks for it — `bind`
    // cannot see the metadata, so the watch is explicit.
    map.set_region(kit_region(&camera.snapshot()), false);
    {
        let map = Retained::clone(map);
        let writing_back = Rc::clone(&writing_back);
        leaf.watch(&camera, move |change| {
            if writing_back.get() {
                return;
            }
            map.set_region(kit_region(change.value()), animated(change.metadata()));
        });
    }
    {
        let map_for_cb = Retained::clone(map);
        let camera_for_cb = camera;
        let observation = map.on_region_change(move || {
            let Some(settled) = map_region(&map_for_cb.region()) else {
                return;
            };
            let current = camera_for_cb.snapshot();
            if regions_close(&current, &settled) {
                return;
            }
            writing_back.set(true);
            camera_for_cb.set(settled);
            writing_back.set(false);
        });
        leaf.keep(observation);
    }
}

/// Wires two-way annotation selection between `config.selection()` and `map`.
fn bind_selection(
    map: &Retained<MapView>,
    leaf: &mut NativeLeaf,
    state: &Rc<RefCell<MapState>>,
    suppressing_selection: &Rc<Cell<bool>>,
    config: &MapConfig,
) {
    let Some(selection) = config.selection().cloned() else {
        return;
    };
    let writing_back = Rc::new(Cell::new(false));
    {
        let mut state = state.borrow_mut();
        apply_selection(
            map,
            &mut state,
            selection.snapshot(),
            false,
            suppressing_selection,
        );
    }
    {
        let map = Retained::clone(map);
        let state = Rc::clone(state);
        let suppressing_selection = Rc::clone(suppressing_selection);
        let writing_back = Rc::clone(&writing_back);
        leaf.watch(&selection, move |change| {
            if writing_back.get() {
                return;
            }
            let mut state = state.borrow_mut();
            apply_selection(
                &map,
                &mut state,
                *change.value(),
                animated(change.metadata()),
                &suppressing_selection,
            );
        });
    }
    {
        let state = Rc::clone(state);
        let suppressing_selection = Rc::clone(suppressing_selection);
        let observation = map.on_annotation_selection(move |event| {
            if suppressing_selection.get() {
                return;
            }
            let next = {
                let mut state = state.borrow_mut();
                match event {
                    AnnotationSelectionEvent::Selected(identity) => {
                        let Some(id) = state.id_for_identity(identity) else {
                            return;
                        };
                        state.selected = Some(id);
                        Some(id)
                    }
                    AnnotationSelectionEvent::Deselected(identity) => {
                        let Some(id) = state.id_for_identity(identity) else {
                            return;
                        };
                        if state.selected != Some(id) {
                            return;
                        }
                        state.selected = None;
                        None
                    }
                    _ => return,
                }
            };
            writing_back.set(true);
            selection.set(next);
            writing_back.set(false);
        });
        leaf.keep(observation);
    }
}

/// The `map` realization: `Native<MapConfig>` maps to
/// a kit `MKMapView` with two-way camera, identity-reconciled selectable
/// annotations, supplied location and load status, plus the one-shot style and
/// visibility flags.
#[expect(
    clippy::needless_pass_by_value,
    reason = "NativeBackend hands each payload by value"
)]
pub fn realize_map_config(config: MapConfig, ctx: &RenderContext<'_>) -> NativeLeaf {
    let map = MapView::new(ctx.mtm());
    map.set_style(kit_style(config.style()));
    map.set_interactive(config.interactivity().is_interactive());
    map.set_shows_compass(config.compass_visibility().is_visible());
    map.set_shows_scale(config.scale_visibility().is_visible());
    map.set_shows_user_location(config.user_location_visibility().is_visible());

    let state = Rc::new(RefCell::new(MapState {
        markers: Vec::new(),
        selected: None,
    }));
    let suppressing_selection = Rc::new(Cell::new(false));

    let mut leaf = NativeLeaf::new(&**map, MapSubView);
    leaf.keep(Rc::clone(&state));
    leaf.keep(Rc::clone(&suppressing_selection));

    bind_camera(&map, &mut leaf, &config);

    // The initial annotation set: the watcher fires on change only.
    {
        let mut state = state.borrow_mut();
        reconcile_annotations(
            &map,
            &mut state,
            &config.resolved_annotations().snapshot(),
            &suppressing_selection,
        );
    }
    {
        let map = map.clone();
        let state = Rc::clone(&state);
        let suppressing_selection = Rc::clone(&suppressing_selection);
        leaf.watch(config.resolved_annotations(), move |change| {
            let mut state = state.borrow_mut();
            reconcile_annotations(&map, &mut state, change.value(), &suppressing_selection);
        });
    }

    bind_selection(&map, &mut leaf, &state, &suppressing_selection, &config);

    // An application-supplied location replaces `CoreLocation` tracking,
    // so the map never shows two disagreeing positions.
    if let Some(location) = config.user_location() {
        map.set_shows_user_location(false);
        let map = map.clone();
        leaf.bind(location, move |value| {
            map.set_supplied_location(value.map(|location| SuppliedLocation {
                coordinate: KitCoordinate::new(
                    location.latitude().get(),
                    location.longitude().get(),
                ),
                horizontal_accuracy: location.horizontal_accuracy(),
            }));
        });
    }

    // `status` is caller-observed only: no binding means no allocation
    // and no delegate beyond what region/location watches installed.
    if let Some(status) = config.status() {
        let status = status.clone();
        let observation = map.on_load_status(move |load| {
            status.set(map_status(&load));
        });
        leaf.keep(observation);
    }

    leaf
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_map::location::{Latitude, Longitude};

    #[test]
    fn kit_coordinate_maps_degrees() {
        let coordinate = Coordinate::new(
            Latitude::new_unchecked(47.6),
            Longitude::new_unchecked(-122.3),
        );
        let kit = kit_coordinate(&coordinate);
        assert_eq!(kit.latitude.to_bits(), 47.6f64.to_bits());
        assert_eq!(kit.longitude.to_bits(), (-122.3f64).to_bits());
    }

    #[test]
    fn kit_region_maps_center_and_span() {
        let region = Region::new(
            Coordinate::new(
                Latitude::new_unchecked(47.6),
                Longitude::new_unchecked(-122.3),
            ),
            0.05,
            0.1,
        );
        let kit = kit_region(&region);
        assert_eq!(kit.center.latitude.to_bits(), 47.6f64.to_bits());
        assert_eq!(kit.span.latitude_delta.to_bits(), 0.05f64.to_bits());
        assert_eq!(kit.span.longitude_delta.to_bits(), 0.1f64.to_bits());
    }

    #[test]
    fn kit_annotation_keeps_optional_subtitle() {
        use waterui_core::id::Mapping;

        let mapping = Mapping::new();
        let id = mapping.to_id(1_u32);
        let coordinate = Coordinate::default();
        let with = kit_annotation(&ResolvedAnnotation::new(
            id,
            coordinate,
            Str::from("title"),
            Some(Str::from("sub")),
        ));
        assert_eq!(with.title, "title");
        assert_eq!(with.subtitle.as_deref(), Some("sub"));
        let without = kit_annotation(&ResolvedAnnotation::new(
            id,
            coordinate,
            Str::from("title"),
            None,
        ));
        assert_eq!(without.subtitle, None);
    }

    #[test]
    fn kit_style_maps_every_variant() {
        assert_eq!(kit_style(MapStyle::Standard), KitStyle::Standard);
        assert_eq!(kit_style(MapStyle::Satellite), KitStyle::Satellite);
        assert_eq!(kit_style(MapStyle::Hybrid), KitStyle::Hybrid);
    }

    #[test]
    fn map_status_maps_load_lifecycle() {
        assert_eq!(map_status(&LoadStatus::Loading), MapStatus::Loading);
        assert_eq!(map_status(&LoadStatus::Ready), MapStatus::Ready);
        match map_status(&LoadStatus::Failed("offline".to_owned())) {
            MapStatus::Failed(reason) => assert_eq!(reason.as_str(), "offline"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_markers_tracks_multiple_added_and_reused_annotations_in_order() {
        use waterui_core::id::Mapping;

        let mapping = Mapping::new();
        let a = mapping.to_id("a");
        let b = mapping.to_id("b");
        let c = mapping.to_id("c");
        let d = mapping.to_id("d");
        let coord = Coordinate::default();

        let mut next_token = 1_usize;
        let mut markers: Vec<(Id, usize)> = Vec::new();

        // Initial mount with three newly added annotations must track all three in order.
        let initial = vec![
            ResolvedAnnotation::new(a, coord, Str::from("A"), None),
            ResolvedAnnotation::new(b, coord, Str::from("B"), None),
            ResolvedAnnotation::new(c, coord, Str::from("C"), None),
        ];
        reconcile_markers(
            &mut markers,
            &initial,
            |_, _| panic!("initial mount has no existing markers to update"),
            |_| panic!("initial mount removes nothing"),
            |added| {
                added
                    .iter()
                    .map(|_| {
                        let token = next_token;
                        next_token += 1;
                        token
                    })
                    .collect()
            },
        );
        assert_eq!(markers, vec![(a, 1), (b, 2), (c, 3)]);

        // Reconcile with a new leading item `d`, reused `b`, removed `a`/`c`, and updated title.
        let mut updated_tokens = Vec::new();
        let mut removed_tokens = Vec::new();
        let next = vec![
            ResolvedAnnotation::new(d, coord, Str::from("D"), None),
            ResolvedAnnotation::new(b, coord, Str::from("B updated"), None),
        ];
        reconcile_markers(
            &mut markers,
            &next,
            |token, annotation| updated_tokens.push((*token, annotation.title.clone())),
            |mut removed| {
                removed.sort_unstable();
                removed_tokens = removed;
            },
            |added| {
                assert_eq!(added.len(), 1);
                let token = next_token;
                next_token += 1;
                vec![token]
            },
        );
        assert_eq!(markers, vec![(d, 4), (b, 2)]);
        assert_eq!(updated_tokens, vec![(2, "B updated".to_owned())]);
        assert_eq!(removed_tokens, vec![1, 3]);
    }
}
