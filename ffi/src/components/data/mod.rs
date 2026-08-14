#[cfg(feature = "map")]
pub mod map;
// The Apple backend names the map's reactive entry points whether or not this
// build has a map, so they exist either way.
#[cfg(all(feature = "c-api", not(feature = "map")))]
pub mod map_absent;
