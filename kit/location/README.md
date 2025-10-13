# waterkit-location

`waterkit-location` provides a cross-platform abstraction for accessing location services within the WaterUI ecosystem. It defines shared data types for working with standard and significant location updates, region monitoring, beacon ranging, and compass headings. Platform specific backends can plug into the abstractions to deliver runtime functionality.

For reactive UIs the crate also exposes [`ReactiveLocationManager`](src/reactive.rs), which wraps a backend in a [`nami`](https://docs.rs/nami) signal graph. Consumers can subscribe to `Binding` handles for the latest samples, monitor bounded event/error histories, or forward events to additional delegates without sacrificing the reactive stream.

The crate ships with an Apple backend that uses [`swift-bridge`](https://crates.io/crates/swift-bridge) to connect to `CoreLocation` and related frameworks. On Android there is a [`jni`](https://docs.rs/jni) powered backend that expects a Java/Kotlin bridge object with methods such as `configureStandardUpdates(json: String)`, `startStandardUpdates()`, and a native companion class (`com.waterkit.location.LocationBridge`) that forwards JSON encoded `LocationEvent` payloads back to Rust. Other platforms can implement the `LocationBackend` trait to offer their own integrations.
