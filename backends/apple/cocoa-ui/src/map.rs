//! The map surface: a typed wrapper over `MapKit`'s `MKMapView`.
//!
//! The wrapper owns the imperative story a consumer needs — region control,
//! annotation handles, an application-supplied location marker with its
//! accuracy circle, and delegate events delivered as Rust closures.
//! `MKMapView` is one class on both platforms, so the surface is unified;
//! `objc2-map-kit` ships `MKMapView` only for `AppKit`, so the `UIKit` side
//! declares the bindings it uses itself.
//!
//! # Safety
//!
//! The `unsafe` here defines an `MKMapView` subclass, an `NSObject`
//! delegate, and (on `UIKit`) a minimal `MKMapView` extern class; it
//! forwards delegate calls `MapKit` makes on the main thread into stored
//! closures, and calls `MapKit`'s own accessors on live objects. All of it
//! is main-thread work, which the `MainThreadOnly` thread kinds and the
//! [`MainThreadMarker`] constructor enforce.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_core_location::CLLocationCoordinate2D;
use objc2_foundation::{NSError, NSObject, NSObjectProtocol, NSString};
use objc2_map_kit::{
    MKCircle, MKCircleRenderer, MKCoordinateRegion, MKCoordinateSpan, MKMapViewDelegate, MKOverlay,
    MKOverlayRenderer, MKPointAnnotation,
};

use crate::callback::guarded;

#[cfg(target_os = "macos")]
use objc2_map_kit::MKMapView;

/// The `UIKit` `MKMapView`: `objc2-map-kit` generates only the `AppKit`
/// class, so this declares the class plus the selectors the wrapper uses.
#[cfg(target_os = "ios")]
mod uikit_map_view {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool, ProtocolObject};
    use objc2::{MainThreadOnly, extern_class, extern_conformance, extern_methods};
    use objc2_core_foundation::CGRect;
    use objc2_foundation::{NSArray, NSCoder, NSObject, NSObjectProtocol};
    use objc2_map_kit::{MKAnnotation, MKCoordinateRegion, MKMapType, MKOverlay};
    use objc2_ui_kit::{UIResponder, UIView};

    extern_class!(
        /// [Apple's documentation](https://developer.apple.com/documentation/mapkit/mkmapview?language=objc)
        #[unsafe(super(UIView, UIResponder, NSObject))]
        #[name = "MKMapView"]
        #[thread_kind = MainThreadOnly]
        #[derive(Debug, PartialEq, Eq, Hash)]
        /// `MapKit`'s map view on `UIKit`.
        pub struct MKMapView;
    );

    extern_conformance!(
        // SAFETY: `NSObjectProtocol` asks nothing of the class.
        unsafe impl NSObjectProtocol for MKMapView {}
    );

    #[allow(non_snake_case)]
    impl MKMapView {
        extern_methods!(
            /// `initWithFrame:`.
            #[unsafe(method(initWithFrame:))]
            #[unsafe(method_family = init)]
            pub unsafe fn initWithFrame(
                this: objc2::rc::Allocated<Self>,
                frame_rect: CGRect,
            ) -> Retained<Self>;

            /// `initWithCoder:`.
            #[unsafe(method(initWithCoder:))]
            #[unsafe(method_family = init)]
            pub unsafe fn initWithCoder(
                this: objc2::rc::Allocated<Self>,
                coder: &NSCoder,
            ) -> Retained<Self>;

            /// The region the map frames.
            #[unsafe(method(region))]
            #[unsafe(method_family = none)]
            pub unsafe fn region(&self) -> MKCoordinateRegion;

            /// Moves the camera to `region`.
            #[unsafe(method(setRegion:animated:))]
            #[unsafe(method_family = none)]
            pub unsafe fn setRegion_animated(&self, region: MKCoordinateRegion, animated: Bool);

            /// The imagery style.
            #[unsafe(method(setMapType:))]
            #[unsafe(method_family = none)]
            pub unsafe fn setMapType(&self, map_type: MKMapType);

            /// Compass visibility.
            #[unsafe(method(setShowsCompass:))]
            #[unsafe(method_family = none)]
            pub unsafe fn setShowsCompass(&self, shows_compass: Bool);

            /// Scale-indicator visibility.
            #[unsafe(method(setShowsScale:))]
            #[unsafe(method_family = none)]
            pub unsafe fn setShowsScale(&self, shows_scale: Bool);

            /// Whether `MapKit` draws the device's own location.
            #[unsafe(method(setShowsUserLocation:))]
            #[unsafe(method_family = none)]
            pub unsafe fn setShowsUserLocation(&self, shows_user_location: Bool);

            /// Adds one annotation.
            #[unsafe(method(addAnnotation:))]
            #[unsafe(method_family = none)]
            pub unsafe fn addAnnotation(&self, annotation: &ProtocolObject<dyn MKAnnotation>);

            /// Adds annotations in bulk.
            #[unsafe(method(addAnnotations:))]
            #[unsafe(method_family = none)]
            pub unsafe fn addAnnotations(
                &self,
                annotations: &NSArray<ProtocolObject<dyn MKAnnotation>>,
            );

            /// Removes one annotation.
            #[unsafe(method(removeAnnotation:))]
            #[unsafe(method_family = none)]
            pub unsafe fn removeAnnotation(&self, annotation: &ProtocolObject<dyn MKAnnotation>);

            /// Removes annotations in bulk.
            #[unsafe(method(removeAnnotations:))]
            #[unsafe(method_family = none)]
            pub unsafe fn removeAnnotations(
                &self,
                annotations: &NSArray<ProtocolObject<dyn MKAnnotation>>,
            );

            /// Adds an overlay above the labels.
            #[unsafe(method(addOverlay:level:))]
            #[unsafe(method_family = none)]
            pub unsafe fn addOverlay_level(
                &self,
                overlay: &ProtocolObject<dyn MKOverlay>,
                level: isize,
            );

            /// Removes an overlay.
            #[unsafe(method(removeOverlay:))]
            #[unsafe(method_family = none)]
            pub unsafe fn removeOverlay(&self, overlay: &ProtocolObject<dyn MKOverlay>);

            /// The map's delegate — `MapKit` holds it weakly.
            #[unsafe(method(setDelegate:))]
            #[unsafe(method_family = none)]
            pub unsafe fn setDelegate(&self, delegate: Option<&AnyObject>);
        );
    }
}

#[cfg(target_os = "ios")]
use uikit_map_view::MKMapView;

/// A geographic coordinate: latitude and longitude in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinate {
    /// Latitude in degrees (-90 to 90).
    pub latitude: f64,
    /// Longitude in degrees (-180 to 180).
    pub longitude: f64,
}

impl Coordinate {
    /// A coordinate from degree values.
    #[must_use]
    pub const fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
        }
    }
}

impl From<Coordinate> for CLLocationCoordinate2D {
    fn from(value: Coordinate) -> Self {
        Self {
            latitude: value.latitude,
            longitude: value.longitude,
        }
    }
}

impl From<CLLocationCoordinate2D> for Coordinate {
    fn from(value: CLLocationCoordinate2D) -> Self {
        Self {
            latitude: value.latitude,
            longitude: value.longitude,
        }
    }
}

/// The north-to-south and east-to-west extent of a [`Region`], in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    /// The north-to-south span in degrees.
    pub latitude_delta: f64,
    /// The east-to-west span in degrees.
    pub longitude_delta: f64,
}

impl Span {
    /// A span from degree values.
    #[must_use]
    pub const fn new(latitude_delta: f64, longitude_delta: f64) -> Self {
        Self {
            latitude_delta,
            longitude_delta,
        }
    }
}

impl From<Span> for MKCoordinateSpan {
    fn from(value: Span) -> Self {
        Self {
            latitudeDelta: value.latitude_delta,
            longitudeDelta: value.longitude_delta,
        }
    }
}

/// A centered region: what the map frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    /// The region's center.
    pub center: Coordinate,
    /// The region's extent.
    pub span: Span,
}

impl Region {
    /// A region from a center and span.
    #[must_use]
    pub const fn new(center: Coordinate, span: Span) -> Self {
        Self { center, span }
    }
}

impl From<Region> for MKCoordinateRegion {
    fn from(value: Region) -> Self {
        Self {
            center: value.center.into(),
            span: value.span.into(),
        }
    }
}

impl From<MKCoordinateRegion> for Region {
    fn from(value: MKCoordinateRegion) -> Self {
        Self {
            center: value.center.into(),
            span: Span::new(value.span.latitudeDelta, value.span.longitudeDelta),
        }
    }
}

/// The imagery a map draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// The standard road map.
    Standard,
    /// Satellite imagery.
    Satellite,
    /// Satellite imagery with road and label overlays.
    Hybrid,
}

/// A pin the map draws: coordinate plus the callout strings.
///
/// An empty or absent subtitle draws no subtitle line.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    /// Where the pin sits.
    pub coordinate: Coordinate,
    /// The pin's title.
    pub title: String,
    /// The pin's subtitle, if any.
    pub subtitle: Option<String>,
}

/// A handle to an [`Annotation`] already on the map. The map owns the
/// annotation object; the handle lets a caller update or remove a specific
/// pin.
#[derive(Debug)]
pub struct AnnotationMarker {
    inner: Retained<MKPointAnnotation>,
}

/// A location the application supplies itself, drawn as the map's own
/// marker: coordinate plus an optional horizontal-accuracy radius in
/// meters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SuppliedLocation {
    /// The location's coordinate.
    pub coordinate: Coordinate,
    /// The horizontal accuracy in meters, drawn as a circle when positive.
    pub horizontal_accuracy: Option<f64>,
}

/// The load lifecycle `MapKit` reports through the delegate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadStatus {
    /// The map started fetching the data it needs to draw.
    Loading,
    /// The map finished loading the current camera's data.
    Ready,
    /// Loading failed, carrying `MapKit`'s localized reason.
    Failed(String),
}

/// Applies an [`Annotation`] to an `MKPointAnnotation`, the shared
/// assignment of add and update.
fn apply(marker: &MKPointAnnotation, annotation: &Annotation) {
    // SAFETY: `MKPointAnnotation` property setters on a live object.
    unsafe {
        marker.setCoordinate(annotation.coordinate.into());
        marker.setTitle(Some(&NSString::from_str(&annotation.title)));
        let subtitle = annotation
            .subtitle
            .as_deref()
            .filter(|text| !text.is_empty())
            .map(NSString::from_str);
        marker.setSubtitle(subtitle.as_deref());
    }
}

type RegionChangedHandler = Rc<dyn Fn()>;
type LoadHandler = Rc<dyn Fn(LoadStatus)>;

/// The closures a [`MapDelegate`] forwards `MapKit` delegate calls to.
#[derive(Default)]
pub struct DelegateIvars {
    region_changed: RefCell<Option<RegionChangedHandler>>,
    load: RefCell<Option<LoadHandler>>,
}

impl fmt::Debug for DelegateIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DelegateIvars")
            .field("region_changed", &self.region_changed.borrow().is_some())
            .field("load", &self.load.borrow().is_some())
            .finish()
    }
}

define_class!(
    // SAFETY: `NSObject` has no designated initializer requirement beyond
    // `init`, which `new` uses, and the class does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiMapDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    #[derive(Debug)]
    /// The `MKMapViewDelegate` that forwards load and region events into
    /// Rust closures.
    pub struct MapDelegate;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for MapDelegate {}

    // SAFETY: `MKMapViewDelegate` is an optional-method protocol; adopting
    // it lets the object become a map view's delegate.
    unsafe impl MKMapViewDelegate for MapDelegate {}

    impl MapDelegate {
        // SAFETY: an optional `MKMapViewDelegate` method, called by `MapKit`
        // on the main thread with live arguments.
        #[unsafe(method(mapView:regionDidChangeAnimated:))]
        fn map_view_region_did_change_animated(&self, _map_view: &MKMapView, _animated: bool) {
            guarded("map region change", || {
                if let Some(handler) = self.ivars().region_changed.borrow().as_ref() {
                    handler();
                }
            });
        }

        // SAFETY: an optional `MKMapViewDelegate` method, called by `MapKit`
        // on the main thread.
        #[unsafe(method(mapViewWillStartLoadingMap:))]
        fn map_view_will_start_loading_map(&self, _map_view: &MKMapView) {
            guarded("map load start", || {
                if let Some(handler) = self.ivars().load.borrow().as_ref() {
                    handler(LoadStatus::Loading);
                }
            });
        }

        // SAFETY: an optional `MKMapViewDelegate` method, called by `MapKit`
        // on the main thread.
        #[unsafe(method(mapViewDidFinishLoadingMap:))]
        fn map_view_did_finish_loading_map(&self, _map_view: &MKMapView) {
            guarded("map load finish", || {
                if let Some(handler) = self.ivars().load.borrow().as_ref() {
                    handler(LoadStatus::Ready);
                }
            });
        }

        // SAFETY: an optional `MKMapViewDelegate` method, called by `MapKit`
        // on the main thread with a live `NSError`.
        #[unsafe(method(mapViewDidFailLoadingMap:withError:))]
        fn map_view_did_fail_loading_map_with_error(
            &self,
            _map_view: &MKMapView,
            error: &NSError,
        ) {
            guarded("map load failure", || {
                if let Some(handler) = self.ivars().load.borrow().as_ref() {
                    handler(LoadStatus::Failed(error.localizedDescription().to_string()));
                }
            });
        }

        // SAFETY: an optional `MKMapViewDelegate` method; `overlay` is a live
        // `MKOverlay` `MapKit` asks a renderer for.
        #[unsafe(method_id(mapView:rendererForOverlay:))]
        fn map_view_renderer_for_overlay(
            &self,
            _map_view: &MKMapView,
            overlay: &ProtocolObject<dyn MKOverlay>,
        ) -> Retained<MKOverlayRenderer> {
            guarded("map overlay renderer", || {
                let object: &AnyObject = overlay.as_ref();
                object.downcast_ref::<MKCircle>().map_or_else(
                    || {
                        // SAFETY: `overlay` is a live `MKOverlay`.
                        unsafe {
                            MKOverlayRenderer::initWithOverlay(
                                MKOverlayRenderer::alloc(),
                                overlay,
                            )
                        }
                    },
                    |circle| {
                        // SAFETY: `circle` is a live `MKCircle`; the returned
                        // renderer is `MKOverlayRenderer`-typed.
                        unsafe {
                            MKCircleRenderer::initWithCircle(MKCircleRenderer::alloc(), circle)
                                .into_super()
                                .into_super()
                        }
                    },
                )
            })
        }
    }
);

impl MapDelegate {
    /// A delegate with no handlers; `MapView` fills the slots in.
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars::default());
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Which handler slot a [`MapObservation`] clears on drop.
#[derive(Debug)]
enum Slot {
    RegionChanged,
    Load,
}

/// A live event subscription on a [`MapView`]; dropping it removes the
/// handler.
#[derive(Debug)]
pub struct MapObservation {
    delegate: Retained<MapDelegate>,
    slot: Slot,
}

impl Drop for MapObservation {
    fn drop(&mut self) {
        match self.slot {
            Slot::RegionChanged => {
                self.delegate.ivars().region_changed.take();
            }
            Slot::Load => {
                self.delegate.ivars().load.take();
            }
        }
    }
}

/// The state a [`MapView`] carries: its delegate and the application-
/// supplied location's marker and accuracy overlay.
pub struct MapViewIvars {
    delegate: RefCell<Option<Retained<MapDelegate>>>,
    location_marker: RefCell<Option<Retained<MKPointAnnotation>>>,
    location_overlay: RefCell<Option<Retained<MKCircle>>>,
}

impl fmt::Debug for MapViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MapViewIvars")
            .field("delegate", &self.delegate.borrow().is_some())
            .field("location_marker", &self.location_marker.borrow().is_some())
            .field(
                "location_overlay",
                &self.location_overlay.borrow().is_some(),
            )
            .finish()
    }
}

define_class!(
    // SAFETY: `MKMapView`'s designated initializer is `initWithFrame:`, which
    // `MapView::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(MKMapView))]
    #[name = "CocoaUiMapView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = MapViewIvars]
    #[derive(Debug)]
    /// An `MKMapView` carrying its delegate and supplied-location overlay.
    pub struct MapView;

    // SAFETY: `NSObjectProtocol` asks nothing of an `MKMapView` subclass.
    unsafe impl NSObjectProtocol for MapView {}
);

impl MapView {
    /// An empty map view.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MapViewIvars {
            delegate: RefCell::new(None),
            location_marker: RefCell::new(None),
            location_overlay: RefCell::new(None),
        });
        // SAFETY: `initWithFrame:` is `MKMapView`'s designated initializer.
        unsafe { self::imp::init(this) }
    }

    /// The view's delegate, created on first subscription. `MKMapView`
    /// holds its delegate weakly, so the view retains it here.
    fn delegate(&self) -> Retained<MapDelegate> {
        if let Some(delegate) = self.ivars().delegate.borrow().as_ref() {
            return delegate.clone();
        }
        let delegate = MapDelegate::new(MainThreadMarker::from(self));
        imp::set_delegate(self, &delegate);
        self.ivars().delegate.replace(Some(delegate.clone()));
        delegate
    }

    /// Calls `handler` whenever the visible region finishes changing —
    /// pans, zooms and programmatic `set_region` calls alike.
    #[must_use]
    pub fn on_region_change(&self, handler: impl Fn() + 'static) -> MapObservation {
        let delegate = self.delegate();
        delegate
            .ivars()
            .region_changed
            .replace(Some(Rc::new(handler)));
        MapObservation {
            delegate,
            slot: Slot::RegionChanged,
        }
    }

    /// Calls `handler` on each load-lifecycle event `MapKit` reports.
    #[must_use]
    pub fn on_load_status(&self, handler: impl Fn(LoadStatus) + 'static) -> MapObservation {
        let delegate = self.delegate();
        delegate.ivars().load.replace(Some(Rc::new(handler)));
        MapObservation {
            delegate,
            slot: Slot::Load,
        }
    }

    /// Moves the map's camera to `region`, animating the transition when
    /// `animated` is set.
    pub fn set_region(&self, region: Region, animated: bool) {
        imp::set_region(self, region, animated);
    }

    /// The region the map currently frames.
    #[must_use]
    pub fn region(&self) -> Region {
        imp::region(self).into()
    }

    /// The imagery the map draws.
    pub fn set_style(&self, style: Style) {
        imp::set_map_type(self, style);
    }

    /// Whether the user can pan, zoom, rotate and pitch the map.
    ///
    /// `UIKit` gates all of it on `userInteractionEnabled`; `AppKit` has no
    /// such switch, so the four gesture flags stand in for it.
    pub fn set_interactive(&self, interactive: bool) {
        imp::set_interactive(self, interactive);
    }

    /// Whether the compass appears when the map is rotated.
    pub fn set_shows_compass(&self, shows: bool) {
        imp::set_shows_compass(self, shows);
    }

    /// Whether the scale indicator appears.
    pub fn set_shows_scale(&self, shows: bool) {
        imp::set_shows_scale(self, shows);
    }

    /// Whether `MapKit` tracks and draws the device's own location.
    pub fn set_shows_user_location(&self, shows: bool) {
        imp::set_shows_user_location(self, shows);
    }

    /// Draws `location` as the map's own marker with an accuracy circle, or
    /// removes both when `None`. A supplied location and
    /// `shows_user_location` are two sources of truth for the same dot —
    /// callers pick one.
    pub fn set_supplied_location(&self, location: Option<SuppliedLocation>) {
        if let Some(overlay) = self.ivars().location_overlay.take() {
            imp::remove_overlay(self, &overlay);
        }
        if let Some(marker) = self.ivars().location_marker.take() {
            imp::remove_annotation(self, &marker);
        }
        let Some(location) = location else {
            return;
        };
        // SAFETY: `alloc`+`init` on `MKPointAnnotation`.
        let marker: Retained<MKPointAnnotation> =
            unsafe { msg_send![MKPointAnnotation::alloc(), init] };
        // SAFETY: `marker` is a live annotation.
        unsafe { marker.setCoordinate(location.coordinate.into()) };
        imp::add_annotation(self, &marker);
        self.ivars().location_marker.replace(Some(marker));

        if let Some(accuracy) = location.horizontal_accuracy
            && accuracy > 0.0
        {
            // SAFETY: `circleWithCenterCoordinate:radius:` is a `MKCircle`
            // class method taking plain geometry.
            let overlay = unsafe {
                MKCircle::circleWithCenterCoordinate_radius(location.coordinate.into(), accuracy)
            };
            imp::add_overlay(self, &overlay);
            self.ivars().location_overlay.replace(Some(overlay));
        }
    }

    /// Adds `annotation`, returning the handle to update or remove it.
    pub fn add_annotation(&self, annotation: &Annotation) -> AnnotationMarker {
        // SAFETY: `alloc`+`init` on `MKPointAnnotation`.
        let marker: Retained<MKPointAnnotation> =
            unsafe { msg_send![MKPointAnnotation::alloc(), init] };
        apply(&marker, annotation);
        imp::add_annotation(self, &marker);
        AnnotationMarker { inner: marker }
    }

    /// Adds each annotation, returning the handles in order.
    pub fn add_annotations(&self, annotations: &[Annotation]) -> Vec<AnnotationMarker> {
        let markers: Vec<Retained<MKPointAnnotation>> = annotations
            .iter()
            .map(|annotation| {
                // SAFETY: `alloc`+`init` on `MKPointAnnotation`.
                let marker: Retained<MKPointAnnotation> =
                    unsafe { msg_send![MKPointAnnotation::alloc(), init] };
                apply(&marker, annotation);
                marker
            })
            .collect();
        imp::add_annotations(self, &markers);
        markers
            .into_iter()
            .map(|inner| AnnotationMarker { inner })
            .collect()
    }

    /// Pushes `annotation`'s values onto an already-rendered marker.
    pub fn update_annotation(&self, marker: &AnnotationMarker, annotation: &Annotation) {
        apply(&marker.inner, annotation);
    }

    /// Removes the annotations `markers` refer to.
    pub fn remove_annotations(&self, markers: &[AnnotationMarker]) {
        imp::remove_annotations(
            self,
            &markers
                .iter()
                .map(|marker| marker.inner.clone())
                .collect::<Vec<_>>(),
        );
    }
}

/// The `AppKit` spelling of the calls: generated `objc2-map-kit` methods.
#[cfg(target_os = "macos")]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{msg_send, rc::PartialInit};
    use objc2_foundation::NSArray;
    use objc2_map_kit::{
        MKAnnotation, MKCircle, MKMapType, MKMapView, MKOverlayLevel, MKPointAnnotation,
    };

    use super::{MapDelegate, MapView, Region, Style};

    /// `initWithFrame:` on the subclass.
    pub(super) unsafe fn init(this: PartialInit<MapView>) -> Retained<MapView> {
        // SAFETY: `initWithFrame:` is `MKMapView`'s designated initializer.
        unsafe { msg_send![super(this), initWithFrame: objc2_core_foundation::CGRect::ZERO] }
    }

    /// `MKMapView`'s weak `delegate`.
    pub(super) fn set_delegate(view: &MapView, delegate: &MapDelegate) {
        // SAFETY: `delegate` is a live `MKMapViewDelegate` object; the
        // property is weak, which the view's ivars compensate by retaining
        // it.
        unsafe {
            view.setDelegate(Some(ProtocolObject::from_ref(delegate)));
        }
    }

    /// `setRegion:animated:`.
    pub(super) fn set_region(view: &MapView, region: Region, animated: bool) {
        // SAFETY: a plain region setter on a live map view.
        unsafe { view.setRegion_animated(region.into(), animated) };
    }

    /// `region`.
    pub(super) fn region(view: &MapView) -> objc2_map_kit::MKCoordinateRegion {
        // SAFETY: a plain region getter on a live map view.
        unsafe { MKMapView::region(view) }
    }

    /// `mapType` — the contract's style spelling; `MKMapConfiguration` is
    /// the modern replacement but reads the same imagery table.
    pub(super) fn set_map_type(view: &MapView, style: Style) {
        let map_type = match style {
            Style::Standard => MKMapType::Standard,
            Style::Satellite => MKMapType::Satellite,
            Style::Hybrid => MKMapType::Hybrid,
        };
        #[allow(deprecated)]
        // SAFETY: a plain setter on a live map view.
        unsafe {
            view.setMapType(map_type);
        }
    }

    /// `NSView` has no `userInteractionEnabled`; the four gesture flags are
    /// the `AppKit` interactivity switch.
    pub(super) fn set_interactive(view: &MapView, interactive: bool) {
        // SAFETY: plain setters on a live map view.
        unsafe {
            view.setZoomEnabled(interactive);
            view.setScrollEnabled(interactive);
            view.setRotateEnabled(interactive);
            view.setPitchEnabled(interactive);
        }
    }

    /// `showsCompass`.
    pub(super) fn set_shows_compass(view: &MapView, shows: bool) {
        // SAFETY: a plain setter on a live map view.
        unsafe { view.setShowsCompass(shows) };
    }

    /// `showsScale`.
    pub(super) fn set_shows_scale(view: &MapView, shows: bool) {
        // SAFETY: a plain setter on a live map view.
        unsafe { view.setShowsScale(shows) };
    }

    /// `showsUserLocation`.
    pub(super) fn set_shows_user_location(view: &MapView, shows: bool) {
        // SAFETY: a plain setter on a live map view.
        unsafe { view.setShowsUserLocation(shows) };
    }

    /// `addAnnotation:`.
    pub(super) fn add_annotation(view: &MapView, marker: &MKPointAnnotation) {
        // SAFETY: `marker` is a live `MKAnnotation`.
        unsafe { view.addAnnotation(ProtocolObject::from_ref(marker)) };
    }

    /// `addAnnotations:`.
    pub(super) fn add_annotations(view: &MapView, markers: &[Retained<MKPointAnnotation>]) {
        if markers.is_empty() {
            return;
        }
        let objects: Vec<Retained<ProtocolObject<dyn MKAnnotation>>> = markers
            .iter()
            .map(|m| ProtocolObject::from_retained(m.clone()))
            .collect();
        let array = NSArray::from_retained_slice(&objects);
        // SAFETY: every element is a live `MKAnnotation`.
        unsafe { view.addAnnotations(&array) };
    }

    /// `removeAnnotation:`.
    pub(super) fn remove_annotation(view: &MapView, marker: &MKPointAnnotation) {
        // SAFETY: `marker` is a live `MKAnnotation` this view added.
        unsafe { view.removeAnnotation(ProtocolObject::from_ref(marker)) };
    }

    /// `removeAnnotations:`.
    pub(super) fn remove_annotations(view: &MapView, markers: &[Retained<MKPointAnnotation>]) {
        if markers.is_empty() {
            return;
        }
        let objects: Vec<Retained<ProtocolObject<dyn MKAnnotation>>> = markers
            .iter()
            .map(|m| ProtocolObject::from_retained(m.clone()))
            .collect();
        let array = NSArray::from_retained_slice(&objects);
        // SAFETY: every element is a live `MKAnnotation` this view added.
        unsafe { view.removeAnnotations(&array) };
    }

    /// `addOverlay:level:` over the labels.
    pub(super) fn add_overlay(view: &MapView, overlay: &MKCircle) {
        // SAFETY: `overlay` is a live `MKOverlay`.
        unsafe {
            view.addOverlay_level(
                ProtocolObject::from_ref(overlay),
                MKOverlayLevel::AboveLabels,
            );
        };
    }

    /// `removeOverlay:`.
    pub(super) fn remove_overlay(view: &MapView, overlay: &MKCircle) {
        // SAFETY: `overlay` is a live `MKOverlay` this view added.
        unsafe { view.removeOverlay(ProtocolObject::from_ref(overlay)) };
    }
}

/// The `UIKit` spelling: `msg_send!` against the hand-declared class.
#[cfg(target_os = "ios")]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, ProtocolObject};
    use objc2::{msg_send, rc::PartialInit};
    use objc2_foundation::NSArray;
    use objc2_map_kit::{MKAnnotation, MKCircle, MKMapType, MKPointAnnotation};

    use super::{MapDelegate, MapView, Region, Style, uikit_map_view::MKMapView};

    /// `MKOverlayLevelAboveLabels`.
    const ABOVE_LABELS: isize = 1;

    /// `initWithFrame:` on the subclass.
    pub(super) unsafe fn init(this: PartialInit<MapView>) -> Retained<MapView> {
        // SAFETY: `initWithFrame:` is `MKMapView`'s designated initializer.
        unsafe { msg_send![super(this), initWithFrame: objc2_core_foundation::CGRect::ZERO] }
    }

    /// `MKMapView`'s weak `delegate`.
    pub(super) fn set_delegate(view: &MapView, delegate: &MapDelegate) {
        // SAFETY: `delegate` is a live `MKMapViewDelegate` object; the
        // property is weak, which the view's ivars compensate by retaining
        // it.
        unsafe { MKMapView::setDelegate(view, Some(delegate.as_ref())) };
    }

    /// `setRegion:animated:`.
    pub(super) fn set_region(view: &MapView, region: Region, animated: bool) {
        // SAFETY: a plain region setter on a live map view.
        unsafe { MKMapView::setRegion_animated(view, region.into(), Bool::new(animated)) };
    }

    /// `region`.
    pub(super) fn region(view: &MapView) -> objc2_map_kit::MKCoordinateRegion {
        // SAFETY: a plain region getter on a live map view.
        unsafe { MKMapView::region(view) }
    }

    /// `mapType`.
    pub(super) fn set_map_type(view: &MapView, style: Style) {
        let map_type = match style {
            Style::Standard => MKMapType::Standard,
            Style::Satellite => MKMapType::Satellite,
            Style::Hybrid => MKMapType::Hybrid,
        };
        // SAFETY: a plain setter on a live map view.
        unsafe { MKMapView::setMapType(view, map_type) };
    }

    /// `UIKit` gates every map gesture on `userInteractionEnabled`.
    pub(super) fn set_interactive(view: &MapView, interactive: bool) {
        // SAFETY: a `UIView` setter on a live map view.
        unsafe {
            let _: () = msg_send![view, setUserInteractionEnabled: Bool::new(interactive)];
        }
    }

    /// `showsCompass`.
    pub(super) fn set_shows_compass(view: &MapView, shows: bool) {
        // SAFETY: a plain setter on a live map view.
        unsafe { MKMapView::setShowsCompass(view, Bool::new(shows)) };
    }

    /// `showsScale`.
    pub(super) fn set_shows_scale(view: &MapView, shows: bool) {
        // SAFETY: a plain setter on a live map view.
        unsafe { MKMapView::setShowsScale(view, Bool::new(shows)) };
    }

    /// `showsUserLocation`.
    pub(super) fn set_shows_user_location(view: &MapView, shows: bool) {
        // SAFETY: a plain setter on a live map view.
        unsafe { MKMapView::setShowsUserLocation(view, Bool::new(shows)) };
    }

    /// `addAnnotation:`.
    pub(super) fn add_annotation(view: &MapView, marker: &MKPointAnnotation) {
        // SAFETY: `marker` is a live `MKAnnotation`.
        unsafe {
            MKMapView::addAnnotation(view, ProtocolObject::from_ref(marker));
        }
    }

    /// `addAnnotations:`.
    pub(super) fn add_annotations(view: &MapView, markers: &[Retained<MKPointAnnotation>]) {
        if markers.is_empty() {
            return;
        }
        let objects: Vec<Retained<ProtocolObject<dyn MKAnnotation>>> = markers
            .iter()
            .map(|m| ProtocolObject::from_retained(m.clone()))
            .collect();
        let array = NSArray::from_retained_slice(&objects);
        // SAFETY: every element is a live `MKAnnotation`.
        unsafe { MKMapView::addAnnotations(view, &array) };
    }

    /// `removeAnnotation:`.
    pub(super) fn remove_annotation(view: &MapView, marker: &MKPointAnnotation) {
        // SAFETY: `marker` is a live `MKAnnotation` this view added.
        unsafe {
            MKMapView::removeAnnotation(view, ProtocolObject::from_ref(marker));
        }
    }

    /// `removeAnnotations:`.
    pub(super) fn remove_annotations(view: &MapView, markers: &[Retained<MKPointAnnotation>]) {
        if markers.is_empty() {
            return;
        }
        let objects: Vec<Retained<ProtocolObject<dyn MKAnnotation>>> = markers
            .iter()
            .map(|m| ProtocolObject::from_retained(m.clone()))
            .collect();
        let array = NSArray::from_retained_slice(&objects);
        // SAFETY: every element is a live `MKAnnotation` this view added.
        unsafe { MKMapView::removeAnnotations(view, &array) };
    }

    /// `addOverlay:level:` over the labels.
    pub(super) fn add_overlay(view: &MapView, overlay: &MKCircle) {
        // SAFETY: `overlay` is a live `MKOverlay`.
        unsafe {
            MKMapView::addOverlay_level(view, ProtocolObject::from_ref(overlay), ABOVE_LABELS);
        }
    }

    /// `removeOverlay:`.
    pub(super) fn remove_overlay(view: &MapView, overlay: &MKCircle) {
        // SAFETY: `overlay` is a live `MKOverlay` this view added.
        unsafe {
            MKMapView::removeOverlay(view, ProtocolObject::from_ref(overlay));
        }
    }
}
