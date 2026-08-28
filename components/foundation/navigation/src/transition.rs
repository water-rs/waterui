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

/// How a retained renderer executes one transition.
///
/// This is deliberately independent of [`NavigationTransition::native`]: a
/// custom transition is free to report a platform-native projection for Apple
/// and Android while still resolving its own frames on a retained renderer.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedNavigationTransition {
    /// Let the renderer apply its own themed platform-default motion.
    PlatformDefault,
    /// Animate geometry matched by this identity across the two pages.
    MatchedGeometry(Id),
    /// Sample [`NavigationTransition::frame`] for every step.
    Frames,
    /// Apply the transaction with no animation at all.
    None,
}

/// A navigation transition executable by retained renderers.
///
/// Apple and Android negotiate [`Self::native`] first. A transition that does
/// not expose a native representation is reported as custom; native backends
/// log a warning and apply the transaction without animation.
///
/// Retained renderers such as Hydrolysis dispatch on [`Self::retained`], which
/// defaults to [`RetainedNavigationTransition::Frames`] — so a custom
/// transition's [`Self::frame`] is always what runs unless it explicitly asks
/// to be rendered some other way.
pub trait NavigationTransition: Debug + 'static {
    /// Resolves a normalized transition frame.
    ///
    /// `progress` is in the inclusive range `0.0..=1.0`. The default is the
    /// identity frame, which is only ever sampled by transitions that opt out
    /// of [`RetainedNavigationTransition::Frames`].
    fn frame(
        &self,
        progress: f32,
        direction: NavigationTransitionDirection,
    ) -> NavigationTransitionFrame {
        let _ = (progress, direction);
        NavigationTransitionFrame::IDENTITY
    }

    /// Returns a platform-native representation when one exists.
    #[doc(hidden)]
    fn native(&self) -> Option<NativeNavigationTransition> {
        None
    }

    /// Returns how a retained renderer should execute this transition.
    #[doc(hidden)]
    fn retained(&self) -> RetainedNavigationTransition {
        RetainedNavigationTransition::Frames
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

    /// Resolves how a retained renderer should execute this transition.
    #[doc(hidden)]
    #[must_use]
    pub fn retained(&self) -> RetainedNavigationTransition {
        self.0.retained()
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

    fn retained(&self) -> RetainedNavigationTransition {
        self.0.retained()
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
        NavigationTransitionFrame, NavigationTransitionLayer, RetainedNavigationTransition,
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

    // `Automatic` deliberately resolves no frames of its own: every backend
    // supplies the motion its platform expects, which for a retained renderer
    // means the themed navigation motion rather than a curve hardcoded here.
    impl NavigationTransition for Automatic {
        fn native(&self) -> Option<NativeNavigationTransition> {
            Some(NativeNavigationTransition::Automatic)
        }

        fn retained(&self) -> RetainedNavigationTransition {
            RetainedNavigationTransition::PlatformDefault
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

        fn retained(&self) -> RetainedNavigationTransition {
            RetainedNavigationTransition::MatchedGeometry(self.source)
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

        fn retained(&self) -> RetainedNavigationTransition {
            RetainedNavigationTransition::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnyNavigationTransition, NativeNavigationTransition, NavigationTransition,
        NavigationTransitionDirection, NavigationTransitionFrame, NavigationTransitionLayer,
        RetainedNavigationTransition, navigation_transition,
    };

    /// A transition that wants the platform's own motion on Apple and Android
    /// but draws its own curve everywhere else — the combination that used to
    /// lose its frames on a retained renderer.
    #[derive(Debug)]
    struct NativelyAutomaticButCustomFrames;

    impl NavigationTransition for NativelyAutomaticButCustomFrames {
        fn frame(
            &self,
            progress: f32,
            _direction: NavigationTransitionDirection,
        ) -> NavigationTransitionFrame {
            NavigationTransitionFrame {
                outgoing: NavigationTransitionLayer {
                    scale: 1.0 - progress,
                    ..NavigationTransitionLayer::IDENTITY
                },
                incoming: NavigationTransitionLayer::IDENTITY,
            }
        }

        fn native(&self) -> Option<NativeNavigationTransition> {
            Some(NativeNavigationTransition::Automatic)
        }
    }

    /// Requesting a platform-native projection must not cost a transition its
    /// own frames: the two capabilities are negotiated independently.
    #[test]
    fn a_native_projection_does_not_suppress_custom_frames() {
        let transition = AnyNavigationTransition::new(NativelyAutomaticButCustomFrames);

        assert_eq!(transition.native(), NativeNavigationTransition::Automatic);
        assert_eq!(transition.retained(), RetainedNavigationTransition::Frames);
        let scale = transition
            .frame(1.0, NavigationTransitionDirection::Push)
            .outgoing
            .scale;
        assert!(
            scale.abs() <= f32::EPSILON,
            "the retained renderer must sample this transition's own curve, got {scale}"
        );
    }

    /// The built-in styles each declare how a retained renderer runs them, so
    /// none of them depend on being recognised by native identity.
    #[test]
    fn built_in_styles_declare_their_retained_execution() {
        assert_eq!(
            AnyNavigationTransition::new(navigation_transition::automatic()).retained(),
            RetainedNavigationTransition::PlatformDefault
        );
        assert_eq!(
            AnyNavigationTransition::new(navigation_transition::fade()).retained(),
            RetainedNavigationTransition::Frames
        );
        assert_eq!(
            AnyNavigationTransition::new(navigation_transition::none()).retained(),
            RetainedNavigationTransition::None
        );
        let id = waterui_core::id::Id::try_from(3).expect("test id must be non-zero");
        assert_eq!(
            AnyNavigationTransition::new(navigation_transition::zoom(id)).retained(),
            RetainedNavigationTransition::MatchedGeometry(id)
        );
    }
}
