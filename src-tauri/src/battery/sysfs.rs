//! Read-only adapter for Linux's `/sys/class/power_supply` hierarchy.
//!
//! This module deliberately exposes provider-specific raw values. The parent
//! battery provider is responsible for translating them into application
//! domain types and for deciding how to combine this source with `UPower`.

use std::{
    fs,
    io::{self, Read},
    path::Path,
};

const MAX_SYSFS_VALUE_BYTES: u64 = 4_096;
const MICRO_PER_BASE_UNIT: f64 = 1_000_000.0;
const MAX_ENERGY_WH: f64 = 1_000_000.0;
const MAX_CHARGE_AH: f64 = 1_000_000.0;
const MAX_POWER_WATTS: f64 = 100_000.0;
const MAX_VOLTAGE_VOLTS: f64 = 100_000.0;
const MAX_CURRENT_AMPS: f64 = 100_000.0;
const MIN_TEMPERATURE_CELSIUS: f64 = -100.0;
const MAX_TEMPERATURE_CELSIUS: f64 = 200.0;
const MAX_CYCLE_COUNT: u64 = 10_000_000;

/// The charging state as reported by a sysfs battery's `status` file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SysfsBatteryStatus {
    /// The battery is receiving charge.
    Charging,
    /// The battery is supplying power.
    Discharging,
    /// The battery reports a full charge.
    Full,
    /// The battery is neither charging nor discharging.
    Idle,
}

/// A raw, normalized reading from one physical sysfs battery.
///
/// Each field is optional because Linux power-supply drivers expose different
/// telemetry. `energy_*` values are expressed in Wh and `charge_*` values in
/// Ah. They intentionally remain separate: their units and meanings cannot be
/// converted without a trustworthy voltage reading.
#[derive(Clone, Debug, PartialEq)]
pub struct SysfsBattery {
    /// Kernel directory name, such as `BAT0`; this is opaque to callers.
    pub id: String,
    /// The battery status when it has a recognized sysfs value.
    pub status: Option<SysfsBatteryStatus>,
    /// Charge level in percent, from zero through one hundred.
    pub capacity_percent: Option<f64>,
    /// Remaining energy in Wh, from `energy_now`.
    pub energy_now_wh: Option<f64>,
    /// Current full energy in Wh, from `energy_full`.
    pub energy_full_wh: Option<f64>,
    /// Design full energy in Wh, from `energy_full_design`.
    pub energy_full_design_wh: Option<f64>,
    /// Remaining charge in Ah, from `charge_now`.
    pub charge_now_ah: Option<f64>,
    /// Current full charge in Ah, from `charge_full`.
    pub charge_full_ah: Option<f64>,
    /// Design full charge in Ah, from `charge_full_design`.
    pub charge_full_design_ah: Option<f64>,
    /// Instantaneous power in W, from `power_now`.
    pub power_watts: Option<f64>,
    /// Voltage in V, from `voltage_now`.
    pub voltage_volts: Option<f64>,
    /// Current in A, from `current_now`.
    pub current_amps: Option<f64>,
    /// Temperature in Celsius, from a tenths-of-a-degree `temp` value.
    pub temperature_celsius: Option<f64>,
    /// Charge cycle count.
    pub cycle_count: Option<u64>,
    /// Battery model name, when the driver exposes one.
    pub model_name: Option<String>,
    /// Battery manufacturer, when the driver exposes one.
    pub manufacturer: Option<String>,
}

/// Reads every power supply whose `type` file contains `Battery`.
///
/// `root` is injectable so callers can use `/sys/class/power_supply` in
/// production and deterministic directory fixtures in tests. A missing root
/// returns its I/O error. An individual malformed, unreadable, or incomplete
/// supply is ignored or represented by `None` fields; it never becomes a
/// fabricated zero measurement.
///
/// # Errors
///
/// Returns an error when the power-supply root cannot be listed.
pub fn read_batteries(root: impl AsRef<Path>) -> io::Result<Vec<SysfsBattery>> {
    let mut batteries = Vec::new();

    for entry in fs::read_dir(root)? {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();

        if read_trimmed(&path.join("type")).as_deref() != Some("Battery") {
            continue;
        }

        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };

        batteries.push(read_battery(&path, id));
    }

    batteries.sort_unstable_by_key(|battery| battery.id.clone());
    Ok(batteries)
}

/// Returns battery identifiers explicitly scoped to a peripheral device.
///
/// Linux exposes wireless mice, keyboards, and similar peripherals as power
/// supplies. They must not be mixed into a laptop's aggregate battery view.
pub fn device_scoped_battery_ids(root: impl AsRef<Path>) -> io::Result<Vec<String>> {
    let mut identifiers = Vec::new();
    for entry in fs::read_dir(root)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if read_trimmed(&path.join("type")).as_deref() != Some("Battery")
            || read_trimmed(&path.join("scope")).as_deref() != Some("Device")
        {
            continue;
        }
        if let Some(id) = entry.file_name().to_str() {
            identifiers.push(id.to_owned());
        }
    }
    identifiers.sort_unstable();
    Ok(identifiers)
}

fn read_battery(path: &Path, id: String) -> SysfsBattery {
    SysfsBattery {
        id,
        status: read_trimmed(&path.join("status"))
            .as_deref()
            .and_then(parse_status),
        capacity_percent: read_unsigned(path, "capacity", 100.0),
        energy_now_wh: read_micro_unsigned(path, "energy_now", MAX_ENERGY_WH),
        energy_full_wh: read_micro_unsigned(path, "energy_full", MAX_ENERGY_WH),
        energy_full_design_wh: read_micro_unsigned(path, "energy_full_design", MAX_ENERGY_WH),
        charge_now_ah: read_micro_unsigned(path, "charge_now", MAX_CHARGE_AH),
        charge_full_ah: read_micro_unsigned(path, "charge_full", MAX_CHARGE_AH),
        charge_full_design_ah: read_micro_unsigned(path, "charge_full_design", MAX_CHARGE_AH),
        power_watts: read_micro_signed(path, "power_now", MAX_POWER_WATTS),
        voltage_volts: read_micro_unsigned(path, "voltage_now", MAX_VOLTAGE_VOLTS)
            .filter(|value| *value > 0.0),
        current_amps: read_micro_signed(path, "current_now", MAX_CURRENT_AMPS),
        temperature_celsius: read_temperature(path),
        cycle_count: read_cycle_count(path),
        model_name: read_trimmed(&path.join("model_name")),
        manufacturer: read_trimmed(&path.join("manufacturer")),
    }
}

fn read_micro_unsigned(path: &Path, name: &str, maximum: f64) -> Option<f64> {
    let value = read_unsigned(path, name, maximum * MICRO_PER_BASE_UNIT)?;
    let normalized = value / MICRO_PER_BASE_UNIT;
    normalized.is_finite().then_some(normalized)
}

fn read_micro_signed(path: &Path, name: &str, maximum: f64) -> Option<f64> {
    let raw_value = read_trimmed(&path.join(name))?;
    if raw_value == "-1" {
        return None;
    }
    let value = parse_f64(&raw_value)? / MICRO_PER_BASE_UNIT;
    (value.is_finite() && value.abs() <= maximum).then_some(value)
}

fn read_unsigned(path: &Path, name: &str, maximum: f64) -> Option<f64> {
    let value = parse_f64(&read_trimmed(&path.join(name))?)?;
    (value.is_finite() && value >= 0.0 && value <= maximum).then_some(value)
}

fn read_temperature(path: &Path) -> Option<f64> {
    let value = parse_f64(&read_trimmed(&path.join("temp"))?)? / 10.0;
    (value.is_finite() && (MIN_TEMPERATURE_CELSIUS..=MAX_TEMPERATURE_CELSIUS).contains(&value))
        .then_some(value)
}

fn read_cycle_count(path: &Path) -> Option<u64> {
    let value = parse_u64(&read_trimmed(&path.join("cycle_count"))?)?;
    (value <= MAX_CYCLE_COUNT).then_some(value)
}

fn parse_status(value: &str) -> Option<SysfsBatteryStatus> {
    match value {
        "Charging" => Some(SysfsBatteryStatus::Charging),
        "Discharging" => Some(SysfsBatteryStatus::Discharging),
        "Full" => Some(SysfsBatteryStatus::Full),
        "Not charging" | "Idle" => Some(SysfsBatteryStatus::Idle),
        _ => None,
    }
}

fn parse_u64(value: &str) -> Option<u64> {
    value.parse().ok()
}

fn parse_f64(value: &str) -> Option<f64> {
    value.parse().ok()
}

fn read_trimmed(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut contents = String::new();
    file.take(MAX_SYSFS_VALUE_BYTES)
        .read_to_string(&mut contents)
        .ok()?;
    let value = contents.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{SysfsBatteryStatus, device_scoped_battery_ids, read_batteries};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

    struct FixtureDirectory(PathBuf);

    impl FixtureDirectory {
        fn new() -> Self {
            let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("battery-dashboard-sysfs-{suffix}"));
            fs::create_dir_all(&path).expect("fixture root can be created");
            Self(path)
        }

        fn supply(&self, name: &str, values: &[(&str, &str)]) {
            let path = self.0.join(name);
            fs::create_dir(&path).expect("supply fixture can be created");
            for (file, value) in values {
                fs::write(path.join(file), value).expect("supply fixture file can be written");
            }
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovers_every_battery_and_keeps_energy_and_charge_separate() {
        let fixture = FixtureDirectory::new();
        fixture.supply(
            "BAT10",
            &[
                ("type", "Battery\n"),
                ("status", "Charging\n"),
                ("capacity", "71\n"),
                ("energy_now", "45123000\n"),
                ("energy_full", "60000000\n"),
                ("energy_full_design", "65000000\n"),
                ("charge_now", "4012000\n"),
                ("charge_full", "5200000\n"),
                ("charge_full_design", "5600000\n"),
                ("power_now", "12345000\n"),
                ("voltage_now", "11200000\n"),
                ("current_now", "-1200000\n"),
                ("temp", "315\n"),
                ("cycle_count", "128\n"),
                ("model_name", " Main Pack \n"),
                ("manufacturer", " Example Cells \n"),
            ],
        );
        fixture.supply(
            "internal-secondary",
            &[
                ("type", "Battery\n"),
                ("status", "Discharging\n"),
                ("capacity", "44\n"),
            ],
        );
        fixture.supply("AC", &[("type", "Mains\n"), ("online", "1\n")]);

        let batteries = read_batteries(fixture.path()).expect("fixture is readable");

        assert_eq!(batteries.len(), 2);
        assert_eq!(batteries[0].id, "BAT10");
        assert_eq!(batteries[0].status, Some(SysfsBatteryStatus::Charging));
        assert_eq!(batteries[0].energy_now_wh, Some(45.123));
        assert_eq!(batteries[0].charge_now_ah, Some(4.012));
        assert_eq!(batteries[0].power_watts, Some(12.345));
        assert_eq!(batteries[0].voltage_volts, Some(11.2));
        assert_eq!(batteries[0].current_amps, Some(-1.2));
        assert_eq!(batteries[0].temperature_celsius, Some(31.5));
        assert_eq!(batteries[0].cycle_count, Some(128));
        assert_eq!(batteries[0].model_name.as_deref(), Some("Main Pack"));
        assert_eq!(batteries[0].manufacturer.as_deref(), Some("Example Cells"));
        assert_eq!(batteries[1].id, "internal-secondary");
        assert_eq!(batteries[1].status, Some(SysfsBatteryStatus::Discharging));
        assert_eq!(batteries[1].energy_now_wh, None);
        assert_eq!(batteries[1].charge_now_ah, None);
    }

    #[test]
    fn keeps_missing_malformed_sentinel_and_out_of_range_values_unavailable() {
        let fixture = FixtureDirectory::new();
        fixture.supply(
            "odd-pack",
            &[
                ("type", "Battery\n"),
                ("status", "Unknown\n"),
                ("capacity", "101\n"),
                ("energy_now", "-1\n"),
                ("energy_full", "not-a-number\n"),
                ("charge_now", "1000000000001\n"),
                ("power_now", "100000000001\n"),
                ("voltage_now", "0\n"),
                ("current_now", "-1\n"),
                ("temp", "3001\n"),
                ("cycle_count", "10000001\n"),
                ("model_name", " \n"),
                ("manufacturer", "\n"),
            ],
        );

        let battery = read_batteries(fixture.path())
            .expect("fixture is readable")
            .pop()
            .expect("battery is discovered");

        assert_eq!(battery.status, None);
        assert_eq!(battery.capacity_percent, None);
        assert_eq!(battery.energy_now_wh, None);
        assert_eq!(battery.energy_full_wh, None);
        assert_eq!(battery.charge_now_ah, None);
        assert_eq!(battery.power_watts, None);
        assert_eq!(battery.voltage_volts, None);
        assert_eq!(battery.current_amps, None);
        assert_eq!(battery.temperature_celsius, None);
        assert_eq!(battery.cycle_count, None);
        assert_eq!(battery.model_name, None);
        assert_eq!(battery.manufacturer, None);
    }

    #[test]
    fn ignores_non_battery_supplies_and_accepts_missing_optional_files() {
        let fixture = FixtureDirectory::new();
        fixture.supply("USB-C", &[("type", "USB\n")]);
        fixture.supply("BAT2", &[("type", "Battery\n")]);

        let batteries = read_batteries(fixture.path()).expect("fixture is readable");

        assert_eq!(batteries.len(), 1);
        assert_eq!(batteries[0].id, "BAT2");
        assert_eq!(batteries[0].capacity_percent, None);
        assert_eq!(batteries[0].status, None);
    }

    #[test]
    fn identifies_peripheral_batteries_without_hiding_laptop_packs() {
        let fixture = FixtureDirectory::new();
        fixture.supply("BAT0", &[("type", "Battery\n"), ("scope", "System\n")]);
        fixture.supply(
            "hidpp_battery_0",
            &[("type", "Battery\n"), ("scope", "Device\n")],
        );

        assert_eq!(
            device_scoped_battery_ids(fixture.path()).expect("fixture is readable"),
            vec!["hidpp_battery_0"]
        );
    }
}
