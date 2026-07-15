//! Extensible navigation transitions.

use core::fmt::Debug;

use waterui_core::{Metadata, View, id::Id, metadata::MetadataKey};

/// Marks the source geometry for a matched navigation transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationTransitionSource(pub(crate) Id);

impl MetadataKey for NavigationTransitionSource {}

/// Marks the destination geometry for a matched navigation transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationTransitionDestination(pub(crate) Id);

impl MetadataKey for NavigationTransitionDestination {}

/// Navigation transition metadata available on every view.
pub trait NavigationTransitionViewExt: View + Sized {
    /// Marks this view as the source geometry for a matched transition.
    fn navigation_transition_source(self, id: Id) -> Metadata<NavigationTransitionSource> {
        Metadata::new(self, NavigationTransitionSource(id))
    }

    /// Marks this view as the destination geometry for a matched transition.
    fn navigation_transition_destination(
        self,
        id: Id,
    ) -> Metadata<NavigationTransitionDestination> {
        Metadata::new(self, NavigationTransitionDestination(id))
    }
}

impl<V: View> NavigationTransitionViewExt for V {}

impl NavigationTransitionSource {
    /// Returns the transition identity.
    #[doc(hidden)]
    #[must_use]
    pub const fn id(self) -> Id {
        self.0
    }
}

impl NavigationTransitionDestination {
    /// Returns the transition identity.
    #[doc(hidden)]
    #[must_use]
    pub const fn id(self) -> Id {
        self.0
    }
}

/// Direction of a navigation transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationTransitionDirection {
    /// A destination is entering above the current page.
    Push,
    /// The current destination is leaving toward its parent.
    Pop,
}

/// Per-layer values resolved by a custom navigation transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavigationTransitionLayer {
    /// Horizontal offset as a fraction of the navigation viewport width.
    pub offset_x: f32,
    /// Vertical offset as a fraction of the navigation viewport height.
    pub offset_y: f32,
    /// Uniform scale.
    pub scale: f32,
    /// Layer opacity.
    pub opacity: f32,
}

impl NavigationTransitionLayer {
    /// An unchanged, fully visible layer.
    pub const IDENTITY: Self = Self {
        offset_x: 0.0,
        offset_y: 0.0,
        scale: 1.0,
        opacity: 1.0,
    };
}

/// One resolved frame of a navigation transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavigationTransitionFrame {
    /// Transform and opacity applied to the page that was active.
    pub outgoing: NavigationTransitionLayer,
    /// Transform and opacity applied to the page becoming active.
    pub incoming: NavigationTransitionLayer,
}

impl NavigationTransitionFrame {
    /// A frame with no visual transition.
    pub const IDENTITY: Self = Self {
        outgoing: NavigationTransitionLayer::IDENTITY,
        incoming: NavigationTransitionLayer::IDENTITY,
    };
}

/// A navigation transition executable by retained renderers.
///
/// Apple and Android negotiate [`Self::native`] first. A transition that does
/// not expose a native representation is reported as custom; native backends
/// log a warning and apply the transaction without animation. Retained
/// renderers such as Hydrolysis execute [`Self::frame`] directly.
pub trait NavigationTransition: Debug + 'static {
    /// Resolves a normalized transition frame.
    ///
    /// `progress` is in the inclusive range `0.0..=1.0`.
    fn frame(
        &self,
        progress: f32,
        direction: NavigationTransitionDirection,
    ) -> NavigationTransitionFrame;

    /// Returns a platform-native representation when one exists.
    #[doc(hidden)]
    fn native(&self) -> Option<NativeNavigationTransition> {
        None
    }
}

/// Type-erased navigation transition retained by a stack.
#[derive(Clone)]
pub struct AnyNavigationTransition(alloc::rc::Rc<dyn NavigationTransition>);

impl Debug for AnyNavigationTransition {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AnyNavigationTransition {
    /// Erases a concrete transition.
    #[must_use]
    pub fn new(transition: impl NavigationTransition) -> Self {
        Self(alloc::rc::Rc::new(transition))
    }

    /// Resolves a frame for a retained renderer.
    ///
    /// # Panics
    ///
    /// Panics unless `progress` is within `0.0..=1.0`.
    #[must_use]
    pub fn frame(
        &self,
        progress: f32,
        direction: NavigationTransitionDirection,
    ) -> NavigationTransitionFrame {
        assert!(
            (0.0..=1.0).contains(&progress),
            "navigation transition progress must be in 0.0..=1.0"
        );
        self.0.frame(progress, direction)
    }

    /// Resolves the native capability projection.
    #[doc(hidden)]
    #[must_use]
    pub fn native(&self) -> NativeNavigationTransition {
        self.0
            .native()
            .unwrap_or(NativeNavigationTransition::Custom)
    }
}

impl NavigationTransition for AnyNavigationTransition {
    fn frame(
        &self,
        progress: f32,
        direction: NavigationTransitionDirection,
    ) -> NavigationTransitionFrame {
        self.0.frame(progress, direction)
    }

    fn native(&self) -> Option<NativeNavigationTransition> {
        self.0.native()
    }
}

/// Transition representation understood by native backends.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeNavigationTransition {
    /// Platform-default navigation motion.
    Automatic,
    /// Cross-fade.
    Fade,
    /// Platform-native zoom/shared-element transition.
    Zoom(Id),
    /// No animation.
    None,
    /// A retained-renderer-only custom transition.
    Custom,
}

/// Built-in navigation transition constructors.
pub mod navigation_transition {
    use super::{
        NativeNavigationTransition, NavigationTransition, NavigationTransitionDirection,
        NavigationTransitionFrame, NavigationTransitionLayer,
    };
    use waterui_core::id::Id;

    /// Platform-default navigation motion.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Automatic;

    /// Cross-fade navigation motion.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Fade;

    /// Platform-native zoom/shared-element navigation motion.
    #[derive(Debug, Clone, Copy)]
    pub struct Zoom {
        source: Id,
    }

    /// Navigation with animation disabled.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct None;

    /// Uses the platform-default navigation motion.
    #[must_use]
    pub const fn automatic() -> Automatic {
        Automatic
    }

    /// Cross-fades between pages.
    #[must_use]
    pub const fn fade() -> Fade {
        Fade
    }

    /// Uses a platform-native zoom/shared-element transition.
    #[must_use]
    pub const fn zoom(source: Id) -> Zoom {
        Zoom { source }
    }

    /// Disables navigation animation.
    #[must_use]
    pub const fn none() -> None {
        None
    }

    impl NavigationTransition for Automatic {
        fn frame(
            &self,
            progress: f32,
            direction: NavigationTransitionDirection,
        ) -> NavigationTransitionFrame {
            let incoming_start = match direction {
                NavigationTransitionDirection::Push => 1.0,
                NavigationTransitionDirection::Pop => -0.25,
            };
            let outgoing_end = match direction {
                NavigationTransitionDirection::Push => -0.25,
                NavigationTransitionDirection::Pop => 1.0,
            };
            NavigationTransitionFrame {
                outgoing: NavigationTransitionLayer {
                    offset_x: outgoing_end * progress,
                    opacity: 1.0 - 0.1 * progress,
                    ..NavigationTransitionLayer::IDENTITY
                },
                incoming: NavigationTransitionLayer {
                    offset_x: incoming_start * (1.0 - progress),
                    ..NavigationTransitionLayer::IDENTITY
                },
            }
        }

        fn native(&self) -> Option<NativeNavigationTransition> {
            Some(NativeNavigationTransition::Automatic)
        }
    }

    impl NavigationTransition for Fade {
        fn frame(
            &self,
            progress: f32,
            _direction: NavigationTransitionDirection,
        ) -> NavigationTransitionFrame {
            NavigationTransitionFrame {
                outgoing: NavigationTransitionLayer {
                    opacity: 1.0 - progress,
                    ..NavigationTransitionLayer::IDENTITY
                },
                incoming: NavigationTransitionLayer {
                    opacity: progress,
                    ..NavigationTransitionLayer::IDENTITY
                },
            }
        }

        fn native(&self) -> Option<NativeNavigationTransition> {
            Some(NativeNavigationTransition::Fade)
        }
    }

    impl NavigationTransition for Zoom {
        fn frame(
            &self,
            progress: f32,
            _direction: NavigationTransitionDirection,
        ) -> NavigationTransitionFrame {
            NavigationTransitionFrame {
                outgoing: NavigationTransitionLayer {
                    scale: 1.0 + 0.05 * progress,
                    opacity: 1.0 - progress,
                    ..NavigationTransitionLayer::IDENTITY
                },
                incoming: NavigationTransitionLayer {
                    scale: 0.85 + 0.15 * progress,
                    opacity: progress,
                    ..NavigationTransitionLayer::IDENTITY
                },
            }
        }

        fn native(&self) -> Option<NativeNavigationTransition> {
            Some(NativeNavigationTransition::Zoom(self.source))
        }
    }

    impl NavigationTransition for None {
        fn frame(
            &self,
            _progress: f32,
            _direction: NavigationTransitionDirection,
        ) -> NavigationTransitionFrame {
            NavigationTransitionFrame::IDENTITY
        }

        fn native(&self) -> Option<NativeNavigationTransition> {
            Some(NativeNavigationTransition::None)
        }
    }
}
