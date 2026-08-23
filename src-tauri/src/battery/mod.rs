//! Composition of live `UPower` and Linux sysfs battery readings.
//!
//! Providers remain independent and are combined only at field level. A
//! missing or unsuitable source value stays unavailable rather than becoming a
//! zero or an invented estimate.

mod sysfs;
mod upower;

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sysfs::{SysfsBattery, SysfsBatteryStatus};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use upower::{UpowerBattery, UpowerState};

const SYSFS_POWER_SUPPLY_ROOT: &str = "/sys/class/power_supply";
const MAX_ENERGY_WH: f64 = 1_000_000.0;
const MAX_POWER_WATTS: f64 = 100_000.0;
const MAX_VOLTAGE_VOLTS: f64 = 100_000.0;
const MAX_CURRENT_AMPS: f64 = 100_000.0;
const MIN_TEMPERATURE_CELSIUS: f64 = -100.0;
const MAX_TEMPERATURE_CELSIUS: f64 = 200.0;

/// The JSON payload consumed by the Svelte battery dashboard.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryDashboardResponse {
    /// Version of the typed desktop payload.
    pub schema_version: u8,
    /// Time at which the providers were queried, in UTC RFC 3339 form.
    pub collected_at: Option<String>,
    /// Whether the complete response is known to be out of date.
    pub stale: bool,
    /// Individual physical batteries discovered at collection time.
    pub batteries: Vec<BatteryResponse>,
}

/// One physical battery in a [`BatteryDashboardResponse`].
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryResponse {
    /// Stable provider identifier, such as `BAT0`.
    pub id: String,
    /// User-readable vendor/model label when hardware provides one.
    pub label: String,
    /// Normalized charging state.
    pub state: &'static str,
    /// Collection timestamp for this battery.
    pub updated_at: Option<String>,
    /// Individual measurements with field-level provenance.
    pub metrics: BatteryMetricsResponse,
}

/// Live metrics for one physical battery.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryMetricsResponse {
    /// Charge percentage.
    pub percentage: MetricResponse,
    /// Remaining energy in watt-hours.
    pub energy_now_wh: MetricResponse,
    /// Current maximum energy in watt-hours.
    pub energy_full_wh: MetricResponse,
    /// Design energy in watt-hours.
    pub energy_design_wh: MetricResponse,
    /// Signed charging/discharging power in watts.
    pub power_watts: MetricResponse,
    /// Voltage in volts.
    pub voltage_volts: MetricResponse,
    /// Signed current in amperes.
    pub current_amps: MetricResponse,
    /// Temperature in degrees Celsius.
    pub temperature_celsius: MetricResponse,
    /// Provider-supported time-to-full or time-to-empty estimate in minutes.
    pub time_remaining_minutes: MetricResponse,
    /// Hardware charge-cycle count.
    pub cycle_count: MetricResponse,
}

/// A single metric with explicit provenance and availability.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricResponse {
    /// Numeric measurement when a provider supplies one.
    pub value: Option<f64>,
    /// Source selected by the composite provider.
    pub source: &'static str,
    /// Availability state of this metric.
    pub availability: &'static str,
    /// Field update time when available.
    pub updated_at: Option<String>,
}

/// Reads and composes the currently available physical batteries.
///
/// Sysfs remains useful when `UPower` is unavailable or does not expose a
/// particular property. A `UPower` connection error is intentionally not fatal:
/// the local sysfs provider can still return useful readings.
#[must_use]
pub async fn read_dashboard() -> BatteryDashboardResponse {
    let timestamp = now_rfc3339();
    let sysfs = sysfs::read_batteries(SYSFS_POWER_SUPPLY_ROOT).unwrap_or_default();
    let upower = upower::enumerate_batteries().await.unwrap_or_default();

    BatteryDashboardResponse {
        schema_version: 1,
        collected_at: timestamp.clone(),
        stale: false,
        batteries: compose_batteries(&sysfs, &upower, timestamp.as_deref()),
    }
}

fn now_rfc3339() -> Option<String> {
    OffsetDateTime::now_utc().format(&Rfc3339).ok()
}

fn compose_batteries(
    sysfs_batteries: &[SysfsBattery],
    upower_batteries: &[UpowerBattery],
    timestamp: Option<&str>,
) -> Vec<BatteryResponse> {
    let sysfs_by_id = sysfs_batteries
        .iter()
        .map(|battery| (battery.id.as_str(), battery))
        .collect::<BTreeMap<_, _>>();
    let upower_by_id = upower_batteries
        .iter()
        .filter_map(|battery| normalized_upower_id(battery).map(|id| (id, battery)))
        .collect::<BTreeMap<_, _>>();
    let ids = sysfs_by_id
        .keys()
        .chain(upower_by_id.keys())
        .copied()
        .collect::<BTreeSet<_>>();

    ids.into_iter()
        .map(|id| {
            compose_battery(
                id,
                sysfs_by_id.get(id).copied(),
                upower_by_id.get(id).copied(),
                timestamp,
            )
        })
        .collect()
}

fn normalized_upower_id(battery: &UpowerBattery) -> Option<&str> {
    battery.native_path.as_deref().and_then(|path| {
        path.rsplit('/')
            .find(|component| !component.is_empty())
            .or_else(|| (!path.trim().is_empty()).then_some(path))
    })
}

fn compose_battery(
    id: &str,
    sysfs: Option<&SysfsBattery>,
    upower: Option<&UpowerBattery>,
    timestamp: Option<&str>,
) -> BatteryResponse {
    let state = select_state(sysfs, upower);
    let percentage = choose_metric(
        upower.and_then(|battery| percentage(battery.percentage)),
        sysfs.and_then(|battery| percentage(battery.capacity_percent)),
        timestamp,
    );
    let energy_now_wh = choose_metric(
        upower.and_then(|battery| non_negative(battery.energy_wh, MAX_ENERGY_WH)),
        sysfs.and_then(|battery| non_negative(battery.energy_now_wh, MAX_ENERGY_WH)),
        timestamp,
    );
    let energy_full_wh = choose_metric(
        upower.and_then(|battery| non_negative(battery.energy_full_wh, MAX_ENERGY_WH)),
        sysfs.and_then(|battery| non_negative(battery.energy_full_wh, MAX_ENERGY_WH)),
        timestamp,
    );
    let energy_design_wh = choose_metric(
        upower.and_then(|battery| non_negative(battery.energy_full_design_wh, MAX_ENERGY_WH)),
        sysfs.and_then(|battery| non_negative(battery.energy_full_design_wh, MAX_ENERGY_WH)),
        timestamp,
    );
    let voltage_volts = choose_metric(
        upower.and_then(|battery| positive(battery.voltage_v, MAX_VOLTAGE_VOLTS)),
        sysfs.and_then(|battery| positive(battery.voltage_volts, MAX_VOLTAGE_VOLTS)),
        timestamp,
    );
    let sysfs_current = sysfs
        .and_then(|battery| signed_for_state(battery.current_amps, state, MAX_CURRENT_AMPS))
        .map(|value| (value, "sysfs"));
    let current_amps = metric_from(sysfs_current, timestamp);
    let power_watts = power_metric(upower, sysfs, state, &voltage_volts, timestamp);
    let temperature_celsius = choose_metric(
        upower.and_then(|battery| temperature(battery.temperature_celsius)),
        sysfs.and_then(|battery| temperature(battery.temperature_celsius)),
        timestamp,
    );
    let time_remaining_minutes = metric_from(
        upower
            .and_then(|battery| time_remaining_minutes(battery, state))
            .map(|value| (value, "upower")),
        timestamp,
    );
    let cycle_count = choose_metric(
        upower.and_then(|battery| non_negative_i32(battery.cycle_count)),
        sysfs.and_then(|battery| {
            battery
                .cycle_count
                .and_then(|value| u32::try_from(value).ok())
                .map(f64::from)
        }),
        timestamp,
    );

    BatteryResponse {
        id: id.to_owned(),
        label: label_for(id, sysfs, upower),
        state,
        updated_at: timestamp.map(str::to_owned),
        metrics: BatteryMetricsResponse {
            percentage,
            energy_now_wh,
            energy_full_wh,
            energy_design_wh,
            power_watts,
            voltage_volts,
            current_amps,
            temperature_celsius,
            time_remaining_minutes,
            cycle_count,
        },
    }
}

fn label_for(id: &str, sysfs: Option<&SysfsBattery>, upower: Option<&UpowerBattery>) -> String {
    let name = upower
        .and_then(|battery| battery.model.as_deref())
        .or_else(|| sysfs.and_then(|battery| battery.model_name.as_deref()))
        .unwrap_or("Battery");
    let vendor = upower
        .and_then(|battery| battery.vendor.as_deref())
        .or_else(|| sysfs.and_then(|battery| battery.manufacturer.as_deref()));

    vendor.map_or_else(
        || format!("{name} ({id})"),
        |vendor| format!("{vendor} {name} ({id})"),
    )
}

fn select_state(sysfs: Option<&SysfsBattery>, upower: Option<&UpowerBattery>) -> &'static str {
    let upower_state = upower
        .and_then(|battery| battery.state)
        .map(map_upower_state);
    upower_state
        .filter(|state| *state != "unknown")
        .or_else(|| sysfs.and_then(|battery| battery.status.map(map_sysfs_status)))
        .unwrap_or("unknown")
}

fn map_upower_state(state: UpowerState) -> &'static str {
    match state {
        UpowerState::Charging | UpowerState::PendingCharge => "charging",
        UpowerState::Discharging | UpowerState::PendingDischarge | UpowerState::Empty => {
            "discharging"
        }
        UpowerState::FullyCharged => "full",
        UpowerState::Unknown => "unknown",
    }
}

fn map_sysfs_status(status: SysfsBatteryStatus) -> &'static str {
    match status {
        SysfsBatteryStatus::Charging => "charging",
        SysfsBatteryStatus::Discharging => "discharging",
        SysfsBatteryStatus::Full => "full",
        SysfsBatteryStatus::Idle => "idle",
    }
}

fn power_metric(
    upower: Option<&UpowerBattery>,
    sysfs: Option<&SysfsBattery>,
    state: &str,
    voltage: &MetricResponse,
    timestamp: Option<&str>,
) -> MetricResponse {
    let upower_power = upower
        .and_then(|battery| signed_for_state(battery.energy_rate_w, state, MAX_POWER_WATTS))
        .map(|value| (value, "upower"));
    let sysfs_power = sysfs
        .and_then(|battery| signed_for_state(battery.power_watts, state, MAX_POWER_WATTS))
        .map(|value| (value, "sysfs"));
    let derived_power = sysfs
        .and_then(|battery| battery.current_amps)
        .zip(voltage.value)
        .and_then(|(current, voltage)| {
            signed_for_state(Some(current.abs() * voltage), state, MAX_POWER_WATTS)
        })
        .map(|value| (value, "derived"));

    metric_from(upower_power.or(sysfs_power).or(derived_power), timestamp)
}

fn time_remaining_minutes(battery: &UpowerBattery, state: &str) -> Option<f64> {
    let seconds = match state {
        "discharging" => battery.time_to_empty_seconds,
        "charging" => battery.time_to_full_seconds,
        _ => None,
    }?;
    let minutes = seconds.checked_add(59)?.div_euclid(60);
    u32::try_from(minutes).ok().map(f64::from)
}

fn signed_for_state(value: Option<f64>, state: &str, maximum: f64) -> Option<f64> {
    let magnitude = value?.abs();
    if !magnitude.is_finite() || magnitude > maximum {
        return None;
    }

    match state {
        "charging" => Some(magnitude),
        "discharging" => Some(-magnitude),
        "full" | "idle" if magnitude == 0.0 => Some(0.0),
        _ => None,
    }
}

fn choose_metric(
    upower: Option<f64>,
    sysfs: Option<f64>,
    timestamp: Option<&str>,
) -> MetricResponse {
    metric_from(
        upower
            .map(|value| (value, "upower"))
            .or_else(|| sysfs.map(|value| (value, "sysfs"))),
        timestamp,
    )
}

fn metric_from(value: Option<(f64, &'static str)>, timestamp: Option<&str>) -> MetricResponse {
    value.map_or_else(MetricResponse::unavailable, |(value, source)| {
        MetricResponse {
            value: Some(value),
            source,
            availability: "available",
            updated_at: timestamp.map(str::to_owned),
        }
    })
}

fn percentage(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
}

fn positive(value: Option<f64>, maximum: f64) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0 && *value <= maximum)
}

fn non_negative(value: Option<f64>, maximum: f64) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value >= 0.0 && *value <= maximum)
}

fn non_negative_i32(value: Option<i32>) -> Option<f64> {
    value.filter(|value| *value >= 0).map(f64::from)
}

fn temperature(value: Option<f64>) -> Option<f64> {
    value.filter(|value| {
        value.is_finite() && (MIN_TEMPERATURE_CELSIUS..=MAX_TEMPERATURE_CELSIUS).contains(value)
    })
}

impl MetricResponse {
    fn unavailable() -> Self {
        Self {
            value: None,
            source: "unavailable",
            availability: "unavailable",
            updated_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MetricResponse, normalized_upower_id, percentage, signed_for_state};
    use crate::battery::upower::UpowerBattery;

    #[test]
    fn keeps_invalid_percentages_unavailable() {
        assert_eq!(percentage(Some(0.0)), Some(0.0));
        assert_eq!(percentage(Some(100.0)), Some(100.0));
        assert_eq!(percentage(Some(100.1)), None);
    }

    #[test]
    fn normalizes_power_direction_from_state_not_driver_sign() {
        assert_eq!(
            signed_for_state(Some(14.5), "discharging", 100.0),
            Some(-14.5)
        );
        assert_eq!(signed_for_state(Some(-14.5), "charging", 100.0), Some(14.5));
        assert_eq!(signed_for_state(Some(3.0), "unknown", 100.0), None);
    }

    #[test]
    fn takes_a_native_path_basename_as_the_cross_provider_identifier() {
        let battery = UpowerBattery {
            id: "/org/freedesktop/UPower/devices/battery_BAT0".to_owned(),
            native_path: Some("/sys/devices/example/power_supply/BAT0".to_owned()),
            vendor: None,
            model: None,
            state: None,
            percentage: None,
            energy_wh: None,
            energy_full_wh: None,
            energy_full_design_wh: None,
            energy_rate_w: None,
            voltage_v: None,
            time_to_empty_seconds: None,
            time_to_full_seconds: None,
            cycle_count: None,
            temperature_celsius: None,
        };

        assert_eq!(normalized_upower_id(&battery), Some("BAT0"));
    }

    #[test]
    fn unavailable_metrics_never_gain_a_zero() {
        let metric = MetricResponse::unavailable();
        assert_eq!(metric.value, None);
        assert_eq!(metric.source, "unavailable");
    }
}
