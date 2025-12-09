//! Toolchain management for `WaterUI` CLI

use std::convert::Infallible;

use color_eyre::eyre;

pub mod cmake;
pub mod doctor;
/// A toolchain that cannot be fixed automatically.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Unfixable toolchain: {message}\n Suggestion: {suggestion}")]
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

/// An error indicating the state of the toolchain.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ToolchainError<Install: Installation> {
    /// The toolchain cannot be fixed automatically.
    #[error("Unfixable toolchain, consider manual intervention")]
    Unfixable(#[from] UnfixableToolchain),
    /// The toolchain is missing components that can be installed.
    #[error("Toolchain is missing, but can be fixed automatically")]
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
    ($($ty:ident),*) => {
        #[allow(unused_variables)]
        #[allow(non_snake_case)]
        impl<$($ty: Toolchain),*> Toolchain for ($($ty,)*) {
            type Installation = ($($ty::Installation,)*);

            async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
                let ($($ty,)*) = self;
                $(
                    match $ty.check().await {
                        Ok(()) => {}
                        Err(e) => {
                            return Err(match e {
                                ToolchainError::Unfixable(u) => ToolchainError::Unfixable(u),
                                ToolchainError::Fixable(_) => ToolchainError::Unfixable(
                                    UnfixableToolchain::new(
                                        format!("One of the toolchains requires fixing"),
                                        "Run the fix command to install missing components",
                                    )
                                ),
                            });
                        }
                    }
                )*
                Ok(())
            }
        }
    };
}

tuples!(impl_toolchains);
