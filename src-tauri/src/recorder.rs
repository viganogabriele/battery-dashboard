//! One-shot collection of live battery readings into local SQLite history.
//!
//! This module is deliberately shared by the desktop shell and the standalone
//! recorder binary. The timer starts the binary, which collects once, commits
//! short database transactions, and exits; it is not a resident daemon.

use std::{error::Error, fmt, fs};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    battery::{BatteryResponse, MetricResponse, read_dashboard},
    storage::{
        InsertOutcome, MetricSource, NewBatterySample, SampleMetric, SampleMetrics, SampleState,
        Storage, StorageError,
    },
};

const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
const UPTIME_PATH: &str = "/proc/uptime";

/// Counts returned after a one-shot recorder execution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RecordSummary {
    /// Number of new immutable rows committed to `SQLite`.
    pub inserted: u32,
    /// Number of rows already present for the same battery and collection instant.
    pub duplicates: u32,
}

/// A failure while converting a live dashboard response into a stored sample.
#[derive(Debug)]
pub enum RecorderError {
    /// The dashboard did not provide an unambiguous UTC collection time.
    InvalidCollectionTimestamp(String),
    /// Linux did not expose a usable boot identifier.
    InvalidBootId(String),
    /// Linux did not expose a usable boot-relative time.
    InvalidBootTime(String),
    /// A battery response did not preserve the live-data contract.
    InvalidDashboardField(String),
    /// `SQLite` storage failed.
    Storage(StorageError),
    /// Reading a local Linux procfs value failed.
    Io(std::io::Error),
}

impl fmt::Display for RecorderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCollectionTimestamp(value) => {
                write!(
                    formatter,
                    "dashboard returned an invalid collection timestamp: {value}"
                )
            }
            Self::InvalidBootId(value) => {
                write!(formatter, "Linux returned an invalid boot ID: {value}")
            }
            Self::InvalidBootTime(value) => {
                write!(
                    formatter,
                    "Linux returned an invalid boot-relative time: {value}"
                )
            }
            Self::InvalidDashboardField(value) => {
                write!(
                    formatter,
                    "dashboard returned an invalid battery field: {value}"
                )
            }
            Self::Storage(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl Error for RecorderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::InvalidCollectionTimestamp(_)
            | Self::InvalidBootId(_)
            | Self::InvalidBootTime(_)
            | Self::InvalidDashboardField(_) => None,
        }
    }
}

impl From<StorageError> for RecorderError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<std::io::Error> for RecorderError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Reads each physical battery once and persists the resulting immutable rows.
///
/// A system with no discoverable batteries is successful with a zero-row
/// summary. The recorder intentionally does not synthesize a desktop battery
/// or a replacement value for any unavailable metric.
///
/// # Errors
///
/// Returns [`RecorderError`] when live data cannot be converted faithfully,
/// Linux boot context cannot be read, or local storage rejects a sample.
pub async fn record_once() -> Result<RecordSummary, RecorderError> {
    let dashboard = read_dashboard().await;
    let recorded_at = parse_collection_timestamp(dashboard.collected_at.as_deref())?;
    let boot_context = BootContext::read()?;
    let samples = dashboard
        .batteries
        .iter()
        .map(|battery| to_sample(battery, recorded_at, &boot_context))
        .collect::<Result<Vec<_>, _>>()?;

    if samples.is_empty() {
        return Ok(RecordSummary::default());
    }

    let mut storage = Storage::open_default()?;
    let mut summary = RecordSummary::default();
    for sample in &samples {
        match storage.insert_sample(sample)? {
            InsertOutcome::Inserted => summary.inserted += 1,
            InsertOutcome::Duplicate => summary.duplicates += 1,
        }
    }
    if summary.inserted > 0 {
        storage.rebuild_sessions()?;
    }

    Ok(summary)
}

#[derive(Clone, Debug, PartialEq)]
struct BootContext {
    id: String,
    seconds: f64,
}

impl BootContext {
    fn read() -> Result<Self, RecorderError> {
        let id = parse_boot_id(&fs::read_to_string(BOOT_ID_PATH)?)?;
        let seconds = parse_boot_seconds(&fs::read_to_string(UPTIME_PATH)?)?;
        Ok(Self { id, seconds })
    }
}

fn parse_collection_timestamp(value: Option<&str>) -> Result<OffsetDateTime, RecorderError> {
    let value = value.ok_or_else(|| {
        RecorderError::InvalidCollectionTimestamp("collection timestamp was unavailable".to_owned())
    })?;
    OffsetDateTime::parse(value, &Rfc3339)
        .map(|timestamp| timestamp.to_offset(time::UtcOffset::UTC))
        .map_err(|error| RecorderError::InvalidCollectionTimestamp(error.to_string()))
}

fn parse_boot_id(value: &str) -> Result<String, RecorderError> {
    let id = value.trim();
    if id.is_empty() || id.contains(char::is_whitespace) {
        return Err(RecorderError::InvalidBootId(
            "empty or malformed value".to_owned(),
        ));
    }
    Ok(id.to_owned())
}

fn parse_boot_seconds(value: &str) -> Result<f64, RecorderError> {
    let value = value.split_whitespace().next().ok_or_else(|| {
        RecorderError::InvalidBootTime("missing first /proc/uptime field".to_owned())
    })?;
    let seconds = value
        .parse::<f64>()
        .map_err(|error| RecorderError::InvalidBootTime(error.to_string()))?;
    if seconds.is_finite() && seconds >= 0.0 {
        Ok(seconds)
    } else {
        Err(RecorderError::InvalidBootTime(
            "value must be finite and non-negative".to_owned(),
        ))
    }
}

fn to_sample(
    battery: &BatteryResponse,
    recorded_at: OffsetDateTime,
    boot_context: &BootContext,
) -> Result<NewBatterySample, RecorderError> {
    Ok(NewBatterySample {
        battery_id: battery.id.clone(),
        recorded_at,
        boot_id: boot_context.id.clone(),
        boot_seconds: boot_context.seconds,
        state: sample_state(battery.state),
        metrics: SampleMetrics {
            percentage: sample_metric(&battery.metrics.percentage)?,
            energy_now_wh: sample_metric(&battery.metrics.energy_now_wh)?,
            energy_full_wh: sample_metric(&battery.metrics.energy_full_wh)?,
            energy_design_wh: sample_metric(&battery.metrics.energy_design_wh)?,
            power_watts: sample_metric(&battery.metrics.power_watts)?,
            voltage_volts: sample_metric(&battery.metrics.voltage_volts)?,
            current_amps: sample_metric(&battery.metrics.current_amps)?,
            temperature_celsius: sample_metric(&battery.metrics.temperature_celsius)?,
            time_remaining_minutes: sample_metric(&battery.metrics.time_remaining_minutes)?,
            cycle_count: sample_metric(&battery.metrics.cycle_count)?,
        },
    })
}

fn sample_state(state: &str) -> SampleState {
    match state {
        "charging" => SampleState::Charging,
        "discharging" => SampleState::Discharging,
        "full" => SampleState::Full,
        "idle" => SampleState::Idle,
        _ => SampleState::Unknown,
    }
}

fn sample_metric(metric: &MetricResponse) -> Result<SampleMetric, RecorderError> {
    match (metric.value, metric.source, metric.availability) {
        (Some(value), "upower", "available") => Ok(SampleMetric {
            value: Some(value),
            source: MetricSource::Upower,
        }),
        (Some(value), "sysfs", "available") => Ok(SampleMetric {
            value: Some(value),
            source: MetricSource::Sysfs,
        }),
        (Some(value), "derived", "available") => Ok(SampleMetric {
            value: Some(value),
            source: MetricSource::Derived,
        }),
        (None, "unavailable", "unavailable") => Ok(SampleMetric::unavailable()),
        _ => Err(RecorderError::InvalidDashboardField(format!(
            "value={:?}, source={}, availability={}",
            metric.value, metric.source, metric.availability
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RecorderError, parse_boot_id, parse_boot_seconds, parse_collection_timestamp, sample_metric,
    };
    use crate::battery::MetricResponse;
    use crate::storage::MetricSource;

    #[test]
    fn parses_boot_context_without_using_wall_clock_guesses() {
        assert_eq!(
            parse_boot_id(" 11111111-2222-3333-4444-555555555555\n")
                .expect("trimmed boot ID is valid"),
            "11111111-2222-3333-4444-555555555555"
        );
        let seconds = parse_boot_seconds("123.45 67.89\n").expect("uptime is valid");
        assert!((seconds - 123.45).abs() < f64::EPSILON);
    }

    #[test]
    fn rejects_missing_or_invalid_clock_values() {
        assert!(matches!(
            parse_collection_timestamp(None),
            Err(RecorderError::InvalidCollectionTimestamp(_))
        ));
        assert!(matches!(
            parse_boot_seconds("not-a-number"),
            Err(RecorderError::InvalidBootTime(_))
        ));
    }

    #[test]
    fn preserves_available_metric_provenance_and_rejects_mixed_contracts() {
        let valid = MetricResponse {
            value: Some(11.2),
            source: "sysfs",
            availability: "available",
            updated_at: None,
        };
        assert_eq!(
            sample_metric(&valid).expect("valid metric maps").source,
            MetricSource::Sysfs
        );

        let invalid = MetricResponse {
            value: None,
            source: "upower",
            availability: "available",
            updated_at: None,
        };
        assert!(matches!(
            sample_metric(&invalid),
            Err(RecorderError::InvalidDashboardField(_))
        ));
    }
}
