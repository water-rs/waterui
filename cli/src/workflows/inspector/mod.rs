//! Inspector app launcher and lifecycle utilities.

mod launcher;

pub use launcher::{
    InspectorLaunchOptions, InspectorPlatform, InspectorSession, launch_inspector_session,
};
