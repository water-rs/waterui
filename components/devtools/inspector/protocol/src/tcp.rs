//! Loopback endpoint configuration shared by the target and the inspector.

use std::net::{IpAddr, Ipv4Addr};
use std::ops::RangeInclusive;

use thiserror::Error;

/// Default host for the local inspector endpoint.
pub const DEFAULT_HOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
/// First port to try.
pub const DEFAULT_PORT_START: u16 = 23106;
/// Number of consecutive ports to try.
pub const DEFAULT_PORT_RANGE: u16 = 20;

/// TCP configuration shared by the target application and the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InspectorTcpConfig {
    /// Address to bind or connect to.
    pub host: IpAddr,
    /// First port to try.
    pub port_start: u16,
    /// Number of consecutive ports to try.
    pub port_range: u16,
}

impl Default for InspectorTcpConfig {
    fn default() -> Self {
        Self::default_localhost()
    }
}

impl InspectorTcpConfig {
    /// Default loopback configuration.
    #[must_use]
    pub const fn default_localhost() -> Self {
        Self {
            host: DEFAULT_HOST,
            port_start: DEFAULT_PORT_START,
            port_range: DEFAULT_PORT_RANGE,
        }
    }

    /// Builds a configuration from environment variables.
    ///
    /// - `WATERUI_INSPECTOR_HOST`
    /// - `WATERUI_INSPECTOR_PORT_START`
    /// - `WATERUI_INSPECTOR_PORT_RANGE`
    ///
    /// # Errors
    ///
    /// Returns an error when a configured variable cannot be parsed. An invalid
    /// value is a mistake worth reporting, not a reason to silently use the
    /// default.
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut config = Self::default_localhost();

        if let Ok(host) = std::env::var("WATERUI_INSPECTOR_HOST") {
            config.host = host.parse().map_err(|_| ConfigError::InvalidHost)?;
        }
        if let Ok(port_start) = std::env::var("WATERUI_INSPECTOR_PORT_START") {
            config.port_start = port_start
                .parse()
                .map_err(|_| ConfigError::InvalidPortStart)?;
        }
        if let Ok(port_range) = std::env::var("WATERUI_INSPECTOR_PORT_RANGE") {
            config.port_range = port_range
                .parse()
                .map_err(|_| ConfigError::InvalidPortRange)?;
        }

        Ok(config)
    }

    /// The inclusive port range to scan or bind.
    #[must_use]
    pub const fn ports(&self) -> RangeInclusive<u16> {
        let end = self
            .port_start
            .saturating_add(self.port_range.saturating_sub(1));
        self.port_start..=end
    }
}

/// Failures parsing the endpoint environment configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// `WATERUI_INSPECTOR_HOST` is not an IP address.
    #[error("invalid WATERUI_INSPECTOR_HOST")]
    InvalidHost,
    /// `WATERUI_INSPECTOR_PORT_START` is not a port number.
    #[error("invalid WATERUI_INSPECTOR_PORT_START")]
    InvalidPortStart,
    /// `WATERUI_INSPECTOR_PORT_RANGE` is not a count.
    #[error("invalid WATERUI_INSPECTOR_PORT_RANGE")]
    InvalidPortRange,
}
