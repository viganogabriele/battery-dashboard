//! Asynchronous, best-effort access to battery devices exposed by `UPower`.
//!
//! This adapter deliberately returns raw per-device readings. Combining those
//! readings with sysfs data, calculating totals, and persistence belong to
//! higher layers.

use std::fmt;

use zbus::{Connection, Proxy, zvariant::OwnedObjectPath};

const UPOWER_SERVICE: &str = "org.freedesktop.UPower";
const UPOWER_ROOT_PATH: &str = "/org/freedesktop/UPower";
const UPOWER_INTERFACE: &str = "org.freedesktop.UPower";
const UPOWER_DEVICE_INTERFACE: &str = "org.freedesktop.UPower.Device";
const DISPLAY_DEVICE_PATH: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

const DEVICE_TYPE_BATTERY: u32 = 2;

/// An error returned while connecting to or querying `UPower`.
#[derive(Debug)]
pub enum UpowerError {
    /// The system D-Bus connection could not be established.
    Connection(zbus::Error),
    /// `UPower` could not enumerate its device object paths.
    Enumeration(zbus::Error),
}

impl fmt::Display for UpowerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection(error) => {
                write!(formatter, "could not connect to system D-Bus: {error}")
            }
            Self::Enumeration(error) => {
                write!(formatter, "could not enumerate UPower devices: {error}")
            }
        }
    }
}

impl std::error::Error for UpowerError {}

/// The charging state reported by `UPower`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpowerState {
    /// `UPower` could not identify the state.
    Unknown,
    /// The device is charging.
    Charging,
    /// The device is discharging.
    Discharging,
    /// The device is empty.
    Empty,
    /// The device is fully charged.
    FullyCharged,
    /// Charging has been requested but has not started yet.
    PendingCharge,
    /// Discharging has been requested but has not started yet.
    PendingDischarge,
}

/// A single physical `UPower` battery device, without aggregation.
#[derive(Clone, Debug, PartialEq)]
pub struct UpowerBattery {
    /// Stable `UPower` object-path identifier for this device.
    pub id: String,
    /// Kernel/native path when `UPower` exposes it.
    pub native_path: Option<String>,
    /// Manufacturer name when exposed by the hardware.
    pub vendor: Option<String>,
    /// Model name when exposed by the hardware.
    pub model: Option<String>,
    /// Current charging state.
    pub state: Option<UpowerState>,
    /// Remaining charge as a percentage.
    pub percentage: Option<f64>,
    /// Remaining energy in watt-hours.
    pub energy_wh: Option<f64>,
    /// Current maximum energy in watt-hours.
    pub energy_full_wh: Option<f64>,
    /// Design maximum energy in watt-hours.
    pub energy_full_design_wh: Option<f64>,
    /// Current energy rate in watts.
    pub energy_rate_w: Option<f64>,
    /// Battery voltage in volts.
    pub voltage_v: Option<f64>,
    /// Seconds until empty while discharging.
    pub time_to_empty_seconds: Option<i64>,
    /// Seconds until full while charging.
    pub time_to_full_seconds: Option<i64>,
    /// Reported charge-cycle count.
    pub cycle_count: Option<i32>,
    /// Battery temperature in degrees Celsius.
    pub temperature_celsius: Option<f64>,
}

/// Lists every physical battery currently exposed by `UPower`.
///
/// The `UPower` `DisplayDevice` is intentionally excluded because it is already
/// an aggregate. A property missing from one device becomes `None`; it does
/// not prevent other devices from being returned.
pub async fn enumerate_batteries() -> Result<Vec<UpowerBattery>, UpowerError> {
    let connection = Connection::system()
        .await
        .map_err(UpowerError::Connection)?;
    enumerate_batteries_on(&connection).await
}

async fn enumerate_batteries_on(
    connection: &Connection,
) -> Result<Vec<UpowerBattery>, UpowerError> {
    let upower = Proxy::new(
        connection,
        UPOWER_SERVICE,
        UPOWER_ROOT_PATH,
        UPOWER_INTERFACE,
    )
    .await
    .map_err(UpowerError::Enumeration)?;
    let paths: Vec<OwnedObjectPath> = upower
        .call("EnumerateDevices", &())
        .await
        .map_err(UpowerError::Enumeration)?;

    let mut batteries = Vec::new();
    for path in paths {
        if is_display_device(path.as_str()) {
            continue;
        }

        if let Some(battery) = read_battery(connection, path).await {
            batteries.push(battery);
        }
    }

    Ok(batteries)
}

async fn read_battery(connection: &Connection, path: OwnedObjectPath) -> Option<UpowerBattery> {
    let device = Proxy::new(
        connection,
        UPOWER_SERVICE,
        path.as_str(),
        UPOWER_DEVICE_INTERFACE,
    )
    .await
    .ok()?;
    let device_type = property::<u32>(&device, "Type").await?;
    if !is_power_supply(device_type, property(&device, "PowerSupply").await) {
        return None;
    }

    Some(UpowerBattery {
        id: path.to_string(),
        native_path: optional_non_empty(property(&device, "NativePath").await),
        vendor: optional_non_empty(property(&device, "Vendor").await),
        model: optional_non_empty(property(&device, "Model").await),
        state: property::<u32>(&device, "State").await.map(upower_state),
        percentage: property(&device, "Percentage").await,
        energy_wh: property(&device, "Energy").await,
        energy_full_wh: property(&device, "EnergyFull").await,
        energy_full_design_wh: property(&device, "EnergyFullDesign").await,
        energy_rate_w: property(&device, "EnergyRate").await,
        voltage_v: property(&device, "Voltage").await,
        time_to_empty_seconds: property(&device, "TimeToEmpty").await,
        time_to_full_seconds: property(&device, "TimeToFull").await,
        cycle_count: property(&device, "ChargeCycles").await,
        temperature_celsius: property(&device, "Temperature").await,
    })
}

async fn property<T>(proxy: &Proxy<'_>, name: &str) -> Option<T>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    <T as TryFrom<zbus::zvariant::OwnedValue>>::Error: Into<zbus::Error>,
{
    proxy.get_property(name).await.ok()
}

fn is_display_device(path: &str) -> bool {
    path == DISPLAY_DEVICE_PATH
}

fn is_power_supply(device_type: u32, power_supply: Option<bool>) -> bool {
    device_type == DEVICE_TYPE_BATTERY && power_supply.unwrap_or(false)
}

fn optional_non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| (!value.trim().is_empty()).then_some(value))
}

fn upower_state(value: u32) -> UpowerState {
    match value {
        1 => UpowerState::Charging,
        2 => UpowerState::Discharging,
        3 => UpowerState::Empty,
        4 => UpowerState::FullyCharged,
        5 => UpowerState::PendingCharge,
        6 => UpowerState::PendingDischarge,
        _ => UpowerState::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEVICE_TYPE_BATTERY, DISPLAY_DEVICE_PATH, UpowerState, is_display_device, is_power_supply,
        optional_non_empty, upower_state,
    };

    #[test]
    fn maps_known_upower_states() {
        assert_eq!(upower_state(1), UpowerState::Charging);
        assert_eq!(upower_state(2), UpowerState::Discharging);
        assert_eq!(upower_state(3), UpowerState::Empty);
        assert_eq!(upower_state(4), UpowerState::FullyCharged);
        assert_eq!(upower_state(5), UpowerState::PendingCharge);
        assert_eq!(upower_state(6), UpowerState::PendingDischarge);
    }

    #[test]
    fn treats_unknown_and_future_states_as_unknown() {
        assert_eq!(upower_state(0), UpowerState::Unknown);
        assert_eq!(upower_state(99), UpowerState::Unknown);
    }

    #[test]
    fn accepts_only_physical_battery_power_supplies() {
        assert!(is_power_supply(DEVICE_TYPE_BATTERY, Some(true)));
        assert!(!is_power_supply(DEVICE_TYPE_BATTERY, Some(false)));
        assert!(!is_power_supply(DEVICE_TYPE_BATTERY, None));
        assert!(!is_power_supply(3, Some(true)));
        assert!(!is_power_supply(4, Some(true)));
    }

    #[test]
    fn removes_blank_optional_text_values() {
        assert_eq!(
            optional_non_empty(Some("BAT0".to_owned())),
            Some("BAT0".to_owned())
        );
        assert_eq!(optional_non_empty(Some("  ".to_owned())), None);
        assert_eq!(optional_non_empty(None), None);
    }

    #[test]
    fn identifies_only_the_upower_aggregate_display_device() {
        assert!(is_display_device(DISPLAY_DEVICE_PATH));
        assert!(!is_display_device(
            "/org/freedesktop/UPower/devices/battery_BAT0"
        ));
    }
}
