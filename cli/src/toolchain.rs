//! Toolchain management for `WaterUI` CLI

use std::convert::Infallible;

use color_eyre::eyre;

pub mod cmake;
pub mod doctor;
pub mod linux;
pub mod meson;
pub mod rust;
pub mod sccache;
pub mod windows_arm64_llvm;
pub mod winget;
pub mod web;
/// A toolchain that cannot be fixed automatically.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Unfixable toolchain: {message}\nSuggestion: {suggestion}")]
pub struct UnfixableToolchain {
    /// A message describing why the toolchain is unfixable.
    message: String,
    /// An suggestion for how to fix the toolchain manually.
    suggestion: String,
}

impl UnfixableToolchain {
    /// Create a new `UnfixableToolchain` with the given message and optional suggestion.
    pub fn new(message: impl Into<String>, suggestion: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            suggestion: suggestion.into(),
        }
    }

    /// Get the message describing why the toolchain is unfixable.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Get the optional suggestion for how to fix the toolchain manually.
    #[must_use]
    pub fn suggestion(&self) -> &str {
        &self.suggestion
    }
}

/// Trait representing an installation plan for toolchain components.
pub trait Installation: Send + Sync {
    /// The error type returned if installation fails.
    type Error: Into<eyre::Report> + Send;
    /// Execute the installation plan.
    fn install(&self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Optional installation step.
///
/// This is used by composite toolchains (e.g. tuples) to represent "install if missing".
impl<I: Installation> Installation for Option<I> {
    type Error = eyre::Report;

    async fn install(&self) -> Result<(), Self::Error> {
        if let Some(install) = self {
            install.install().await.map_err(Into::into)?;
        }
        Ok(())
    }
}

/// An error indicating the state of the toolchain.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ToolchainError<Install: Installation> {
    /// The toolchain cannot be fixed automatically.
    #[error("{0}")]
    Unfixable(#[from] UnfixableToolchain),
    /// The toolchain is missing components that can be installed.
    #[error(
        "Toolchain is missing components that can be fixed automatically. Run `water doctor --fix` for details."
    )]
    Fixable(Install),
}

impl<I: Installation> ToolchainError<I> {
    /// Returns `true` if the toolchain can be fixed automatically.
    #[must_use]
    pub const fn is_fixable(&self) -> bool {
        matches!(self, Self::Fixable(_))
    }

    /// Create a new `ToolchainError` indicating that the toolchain can be fixed automatically.
    #[must_use]
    pub const fn fixable(install: I) -> Self {
        Self::Fixable(install)
    }

    /// Create a new `ToolchainError` indicating that the toolchain cannot be fixed automatically.
    #[must_use]
    pub fn unfixable(message: impl Into<String>, suggestion: impl Into<String>) -> Self {
        Self::Unfixable(UnfixableToolchain::new(message, suggestion))
    }
}

/// Trait for toolchain dependencies that can be checked and installed.
///
/// Implementors represent a specific toolchain configuration (e.g., Rust with
/// certain targets, Android SDK with specific components).
/// The associated `Installation` type preserves full type information through
/// the composition, enabling zero-cost abstractions for parallel/sequential
/// installation plans.
pub trait Toolchain: Send + Sync {
    /// The installation type returned by `fix()`.
    type Installation: Installation;

    /// Check if the toolchain is properly installed.
    ///
    /// Returns `Ok(())` if all components are available, or `Err` describing
    /// what is missing.
    fn check(&self) -> impl Future<Output = Result<(), ToolchainError<Self::Installation>>> + Send;
}

impl Installation for Infallible {
    type Error = Self;

    async fn install(&self) -> Result<(), Self::Error> {
        unreachable!()
    }
}

impl Toolchain for Infallible {
    type Installation = Self;

    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        unreachable!()
    }
}

macro_rules! tuples {
    ($macro:ident) => {
        $macro!();
        $macro!(T0);
        $macro!(T0, T1);
        $macro!(T0, T1, T2);
        $macro!(T0, T1, T2, T3);
        $macro!(T0, T1, T2, T3, T4);
        $macro!(T0, T1, T2, T3, T4, T5);
        $macro!(T0, T1, T2, T3, T4, T5, T6);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13);
        $macro!(
            T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14
        );
    };
}

macro_rules! impl_installations {
    ($($ty:ident),*) => {
        #[allow(unused_variables)]
        #[allow(non_snake_case)]
        impl<$($ty: Installation),*> Installation for ($($ty,)*) {
            type Error = eyre::Report;
            async fn install(&self) -> Result<(), Self::Error> {
                let ($($ty,)*) = self;
                $(
                    $ty.install().await.map_err(|e| e.into())?;
                )*
                Ok(())
            }
        }
    };
}

tuples!(impl_installations);

macro_rules! impl_toolchains {
    ($(($idx:tt, $ty:ident)),*) => {
        #[allow(unused_variables)]
        #[allow(non_snake_case)]
        impl<$($ty: Toolchain),*> Toolchain for ($($ty,)*) {
            // Each slot is `Some(install)` if that component is missing-and-fixable.
            // Components that are already OK produce `None` and are skipped during installation.
            type Installation = ($(Option<$ty::Installation>,)*);

            async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
                #[allow(unused_mut)]
                let mut any_fixable = false;
                #[allow(unused_mut)]
                let mut installs: Self::Installation = ($(None::<$ty::Installation>,)*);

                $(
                    match self.$idx.check().await {
                        Ok(()) => {}
                        Err(ToolchainError::Unfixable(u)) => {
                            return Err(ToolchainError::Unfixable(u));
                        }
                        Err(ToolchainError::Fixable(install)) => {
                            any_fixable = true;
                            installs.$idx = Some(install);
                        }
                    }
                )*

                if any_fixable {
                    Err(ToolchainError::Fixable(installs))
                } else {
                    Ok(())
                }
            }
        }
    };
}

macro_rules! tuples_idx {
    ($macro:ident) => {
        $macro!();
        $macro!((0, T0));
        $macro!((0, T0), (1, T1));
        $macro!((0, T0), (1, T1), (2, T2));
        $macro!((0, T0), (1, T1), (2, T2), (3, T3));
        $macro!((0, T0), (1, T1), (2, T2), (3, T3), (4, T4));
        $macro!((0, T0), (1, T1), (2, T2), (3, T3), (4, T4), (5, T5));
        $macro!(
            (0, T0),
            (1, T1),
            (2, T2),
            (3, T3),
            (4, T4),
            (5, T5),
            (6, T6)
        );
        $macro!(
            (0, T0),
            (1, T1),
            (2, T2),
            (3, T3),
            (4, T4),
            (5, T5),
            (6, T6),
            (7, T7)
        );
        $macro!(
            (0, T0),
            (1, T1),
            (2, T2),
            (3, T3),
            (4, T4),
            (5, T5),
            (6, T6),
            (7, T7),
            (8, T8)
        );
        $macro!(
            (0, T0),
            (1, T1),
            (2, T2),
            (3, T3),
            (4, T4),
            (5, T5),
            (6, T6),
            (7, T7),
            (8, T8),
            (9, T9)
        );
        $macro!(
            (0, T0),
            (1, T1),
            (2, T2),
            (3, T3),
            (4, T4),
            (5, T5),
            (6, T6),
            (7, T7),
            (8, T8),
            (9, T9),
            (10, T10)
        );
        $macro!(
            (0, T0),
            (1, T1),
            (2, T2),
            (3, T3),
            (4, T4),
            (5, T5),
            (6, T6),
            (7, T7),
            (8, T8),
            (9, T9),
            (10, T10),
            (11, T11)
        );
        $macro!(
            (0, T0),
            (1, T1),
            (2, T2),
            (3, T3),
            (4, T4),
            (5, T5),
            (6, T6),
            (7, T7),
            (8, T8),
            (9, T9),
            (10, T10),
            (11, T11),
            (12, T12)
        );
        $macro!(
            (0, T0),
            (1, T1),
            (2, T2),
            (3, T3),
            (4, T4),
            (5, T5),
            (6, T6),
            (7, T7),
            (8, T8),
            (9, T9),
            (10, T10),
            (11, T11),
            (12, T12),
            (13, T13)
        );
        $macro!(
            (0, T0),
            (1, T1),
            (2, T2),
            (3, T3),
            (4, T4),
            (5, T5),
            (6, T6),
            (7, T7),
            (8, T8),
            (9, T9),
            (10, T10),
            (11, T11),
            (12, T12),
            (13, T13),
            (14, T14)
        );
    };
}

tuples_idx!(impl_toolchains);
