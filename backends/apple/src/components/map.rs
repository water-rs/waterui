//! The `map` leaf: `Native<MapConfig>` rendered through the kit's `MKMapView`
//! wrapper.
//!
//! Mirrors `WuiMapComponent`: the `region` `Computed<Region>` pushes
//! `setRegion` (animated when the watcher metadata carries an `Animation`),
//! `annotations` reconciles by index — rendered pins are reused in place,
//! the tail removed, extras appended — an application-supplied
//! `user_location` replaces `showsUserLocation` tracking with the map's own
//! marker and accuracy circle, and a `status` `Binding` receives `MapKit`'s
//! load lifecycle. Every update is an imperative kit call inside a watcher —
//! no signal type crosses into `cocoa-ui`.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use cocoa_ui::map::{AnnotationMarker, LoadStatus, MapView, Style as KitStyle, SuppliedLocation};
use waterui::Str;
use waterui::animation::Animation;
use waterui::reactive::Signal;
use waterui::reactive::watcher::Metadata;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
use waterui_map::{Annotation, Coordinate, MapConfig, MapStatus, MapStyle, Region};

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

/// Whether this watcher callback animates: the metadata carries an
/// `Animation`, the same check `metadata.animation != nil` performed.
fn animated(metadata: &Metadata) -> bool {
    metadata.try_get::<Animation>().is_some()
}

/// A `waterui_map::Coordinate` as the kit's degree-pair.
const fn kit_coordinate(coordinate: &Coordinate) -> cocoa_ui::map::Coordinate {
    cocoa_ui::map::Coordinate::new(coordinate.latitude.get(), coordinate.longitude.get())
}

/// A `waterui_map::Region` as the kit's center-and-span region.
const fn kit_region(region: &Region) -> cocoa_ui::map::Region {
    cocoa_ui::map::Region::new(
        kit_coordinate(&region.center),
        cocoa_ui::map::Span::new(region.latitude_delta, region.longitude_delta),
    )
}

/// A `waterui_map::Annotation` as the kit's pin value.
fn kit_annotation(annotation: &Annotation) -> cocoa_ui::map::Annotation {
    cocoa_ui::map::Annotation {
        coordinate: kit_coordinate(&annotation.coordinate),
        title: annotation.title.as_str().to_owned(),
        subtitle: annotation
            .subtitle
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

/// The leaf's live state: the markers currently on the map, reconciled
/// against `config.annotations` on every change. `KeepAlive` holds the `Rc`.
#[derive(Debug)]
struct MapState {
    /// The rendered annotation handles, in `annotations` order.
    markers: Vec<AnnotationMarker>,
}

/// Applies `values` onto the map, reusing rendered markers by index —
/// `reconcileAnnotations` in the Swift port.
fn reconcile_annotations(view: &MapView, state: &mut MapState, values: &[Annotation]) {
    let reused = state.markers.len().min(values.len());
    for (marker, value) in state.markers.iter().zip(values.iter()).take(reused) {
        view.update_annotation(marker, &kit_annotation(value));
    }

    if state.markers.len() > values.len() {
        let removed = state.markers.split_off(values.len());
        view.remove_annotations(&removed);
    } else if values.len() > state.markers.len() {
        let added = view.add_annotations(
            &values[state.markers.len()..]
                .iter()
                .map(kit_annotation)
                .collect::<Vec<_>>(),
        );
        state.markers.extend(added);
    }
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

/// Installs the `map` handler on the dispatcher: `Native<MapConfig>` maps to
/// a kit `MKMapView` with reactive region, annotations, supplied location and
/// load status, plus the one-shot style and visibility flags.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<MapConfig>(|config, ctx| {
        let map = MapView::new(ctx.mtm());
        map.set_style(kit_style(config.style));
        map.set_interactive(config.interactivity.is_interactive());
        map.set_shows_compass(config.compass_visibility.is_visible());
        map.set_shows_scale(config.scale_visibility.is_visible());
        map.set_shows_user_location(config.user_location_visibility.is_visible());

        let state = Rc::new(RefCell::new(MapState {
            markers: Vec::new(),
        }));

        let mut leaf = NativeLeaf::new(&**map, MapSubView);
        leaf.keep(Rc::clone(&state));

        // The region snapshot applies unanimated at mount; each change
        // animates only when the watcher metadata asks for it — `bind`
        // cannot see the metadata, so the watch is explicit.
        map.set_region(kit_region(&config.region.snapshot()), false);
        {
            let map = map.clone();
            leaf.watch(&config.region, move |change| {
                map.set_region(kit_region(change.value()), animated(change.metadata()));
            });
        }

        // The initial annotation set: the watcher fires on change only.
        {
            let mut state = state.borrow_mut();
            reconcile_annotations(&map, &mut state, &config.annotations.snapshot());
        }
        {
            let map = map.clone();
            let state = Rc::clone(&state);
            leaf.watch(&config.annotations, move |change| {
                let mut state = state.borrow_mut();
                reconcile_annotations(&map, &mut state, change.value());
            });
        }

        // An application-supplied location replaces `CoreLocation` tracking,
        // so the map never shows two disagreeing positions.
        if let Some(location) = &config.user_location {
            map.set_shows_user_location(false);
            let map = map.clone();
            leaf.bind(location, move |value| {
                map.set_supplied_location(value.map(|location| SuppliedLocation {
                    coordinate: cocoa_ui::map::Coordinate::new(
                        location.latitude().get(),
                        location.longitude().get(),
                    ),
                    horizontal_accuracy: location.horizontal_accuracy(),
                }));
            });
        }

        // `status` is caller-observed only: no binding means no allocation
        // and no delegate beyond what region/location watches installed.
        if let Some(status) = &config.status {
            let status = status.clone();
            let observation = map.on_load_status(move |load| {
                status.set(map_status(&load));
            });
            leaf.keep(observation);
        }

        leaf
    });
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
        let coordinate = Coordinate::default();
        let with = kit_annotation(&Annotation::new(coordinate, "title").subtitle("sub"));
        assert_eq!(with.title, "title");
        assert_eq!(with.subtitle.as_deref(), Some("sub"));
        let without = kit_annotation(&Annotation::new(coordinate, "title"));
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
}
