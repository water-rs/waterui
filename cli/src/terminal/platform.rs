use core::fmt::Display;

use clap::ValueEnum;

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum Platform {
    Web,
    #[clap(alias = "mac")]
    Macos,
    #[clap(alias = "iphone")]
    Ios,
    #[clap(alias = "ipad")]
    Ipados,
    #[clap(alias = "watch")]
    Watchos,
    #[clap(alias = "tv")]
    Tvos,
    #[clap(alias = "vision")]
    Visionos,
    Android,
}

impl Platform {
    pub fn is_apple_platform(&self) -> bool {
        matches!(
            self,
            Platform::Macos
                | Platform::Ios
                | Platform::Ipados
                | Platform::Watchos
                | Platform::Tvos
                | Platform::Visionos
        )
    }

    pub fn is_mobile_platform(&self) -> bool {
        matches!(
            self,
            Platform::Ios
                | Platform::Ipados
                | Platform::Watchos
                | Platform::Tvos
                | Platform::Android
        )
    }

    pub fn check_toolchain(&self) {}
}

impl Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Platform::Web => write!(f, "Web"),
            Platform::Macos => write!(f, "macOS"),
            Platform::Ios => write!(f, "iOS"),
            Platform::Ipados => write!(f, "iPadOS"),
            Platform::Watchos => write!(f, "watchOS"),
            Platform::Tvos => write!(f, "tvOS"),
            Platform::Visionos => write!(f, "visionOS"),
            Platform::Android => write!(f, "Android"),
        }
    }
}
