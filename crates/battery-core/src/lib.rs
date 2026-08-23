//! Shared, platform-neutral domain types for Battery Dashboard.
//!
//! This crate deliberately contains no operating-system integration, storage,
//! scheduling, or UI code. Platform adapters will translate their readings into
//! these types in later development phases.

#![forbid(unsafe_code)]

use std::fmt;

/// The current charging state reported by a battery provider.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BatteryState {
    /// The battery is receiving energy.
    Charging,
    /// The battery is supplying energy.
    Discharging,
    /// The battery reports that it is fully charged.
    Full,
    /// The battery is neither charging nor discharging.
    Idle,
    /// The provider cannot determine the state.
    #[default]
    Unknown,
}

impl BatteryState {
    /// Returns whether this state describes energy transfer.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Charging | Self::Discharging)
    }
}

/// An identifier for one physical battery as supplied by a platform adapter.
///
/// The string is intentionally opaque: adapters may use names such as `BAT0`,
/// but consumers must not derive platform-specific meaning from it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BatteryId(String);

impl BatteryId {
    /// Creates an identifier when `value` is non-empty after trimming.
    ///
    /// The original value is preserved, except that leading and trailing
    /// whitespace is removed.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBatteryId`] when the trimmed value is empty.
    pub fn new(value: impl AsRef<str>) -> Result<Self, InvalidBatteryId> {
        let value = value.as_ref().trim();
        if value.is_empty() {
            return Err(InvalidBatteryId);
        }

        Ok(Self(value.to_owned()))
    }

    /// Returns the adapter-provided identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BatteryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The error returned when a [`BatteryId`] is empty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidBatteryId;

impl fmt::Display for InvalidBatteryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("battery identifier cannot be empty")
    }
}

impl std::error::Error for InvalidBatteryId {}

/// A platform-neutral instant reading for one battery.
///
/// Fields are optional because hardware and Linux drivers expose different
/// subsets of telemetry. A missing value must remain missing; callers must not
/// substitute zero or infer a measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct BatteryReading {
    /// Physical battery identifier.
    pub id: BatteryId,
    /// Current charge in percent, if reported by the provider.
    pub percentage: Option<f64>,
    /// Current charge/discharge state.
    pub state: BatteryState,
    /// Signed instantaneous power in watts, if reported.
    pub power_watts: Option<f64>,
    /// Remaining energy in watt-hours, if reported.
    pub energy_wh: Option<f64>,
}

impl BatteryReading {
    /// Returns `true` when all numeric values in the reading are finite and
    /// percentage is within the inclusive range from zero to one hundred.
    #[must_use]
    pub fn has_valid_measurements(&self) -> bool {
        self.percentage
            .is_none_or(|value| value.is_finite() && (0.0..=100.0).contains(&value))
            && self.power_watts.is_none_or(f64::is_finite)
            && self
                .energy_wh
                .is_none_or(|value| value.is_finite() && value >= 0.0)
    }
}

/// Returns the crate version embedded at build time.
///
/// This is intentionally exposed for diagnostic screens and smoke tests.
#[must_use]
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::{BatteryId, BatteryReading, BatteryState, version};

    #[test]
    fn domain_smoke_test() {
        let id = BatteryId::new(" BAT0 ").expect("a non-empty identifier is valid");
        let reading = BatteryReading {
            id,
            percentage: Some(73.5),
            state: BatteryState::Discharging,
            power_watts: Some(8.2),
            energy_wh: Some(41.1),
        };

        assert_eq!(reading.id.as_str(), "BAT0");
        assert!(reading.state.is_active());
        assert!(reading.has_valid_measurements());
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn missing_values_remain_valid_but_invalid_measurements_are_rejected() {
        let id = BatteryId::new("BAT1").expect("a non-empty identifier is valid");
        let reading = BatteryReading {
            id,
            percentage: None,
            state: BatteryState::Unknown,
            power_watts: Some(f64::NAN),
            energy_wh: None,
        };

        assert!(!reading.has_valid_measurements());
        assert!(BatteryId::new(" \t ").is_err());
    }

    #[test]
    fn negative_energy_is_rejected_without_rejecting_signed_power() {
        let id = BatteryId::new("internal-battery").expect("a non-empty identifier is valid");
        let reading = BatteryReading {
            id,
            percentage: Some(50.0),
            state: BatteryState::Charging,
            power_watts: Some(-12.5),
            energy_wh: Some(-1.0),
        };

        assert!(!reading.has_valid_measurements());
    }
}
