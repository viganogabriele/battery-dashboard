//! Privacy-safe, explicit-path exports of local battery history.
//!
//! This module deliberately has no dialog or application-directory policy. A
//! caller must provide the destination selected by the user. Exports never
//! contain hardware serial numbers (which are not part of the export schema).

use std::{
    error::Error,
    fmt,
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::storage::{BatterySession, HistoryMetric, HistorySample, SessionAggregation};

/// Stable version of the on-disk export schemas.
pub const EXPORT_SCHEMA_VERSION: u32 = 1;

/// File representation selected by the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportFormat {
    /// RFC 4180-compatible comma-separated values.
    Csv,
    /// UTF-8 JSON with typed numeric values and `null` for missing values.
    Json,
}

impl ExportFormat {
    /// Suggested filename extension, without a dot.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

/// Export-wide metadata. All fields are supplied by the caller so that a
/// command can use its selected timezone and a deterministic test can use a
/// fixed generation time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportMetadata {
    /// RFC 3339 generation timestamp.
    pub generated_at: String,
    /// IANA timezone identifier used by the caller for calendar summaries.
    pub timezone: String,
}

/// Exactly one stable record family in an export.
#[derive(Clone, Debug, PartialEq)]
pub enum ExportRecords {
    /// Immutable raw observations. Boot identifiers are deliberately omitted.
    RawSamples(Vec<HistorySample>),
    /// Rebuildable, derived contiguous activities.
    Sessions(Vec<BatterySession>),
    /// Daily, weekly, or monthly derived session buckets.
    Summaries(Vec<SessionAggregation>),
}

/// A complete export prepared by the command layer.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportDocument {
    /// Schema and contextual metadata.
    pub metadata: ExportMetadata,
    /// Data family and records to encode.
    pub records: ExportRecords,
}

/// Failure while rendering or safely writing an explicit export destination.
#[derive(Debug)]
pub enum ExportError {
    /// The destination is not an explicit filename in an existing directory.
    InvalidPath(String),
    /// The destination already exists; exports never replace a file.
    DestinationExists(PathBuf),
    /// An operating-system write, sync, or rename operation failed.
    Io(io::Error),
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(message) => write!(formatter, "invalid export path: {message}"),
            Self::DestinationExists(path) => write!(
                formatter,
                "refusing to overwrite existing export destination {}",
                path.display()
            ),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl Error for ExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::InvalidPath(_) | Self::DestinationExists(_) => None,
        }
    }
}

impl From<io::Error> for ExportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Renders a document without touching the filesystem.
///
/// # Errors
///
/// This currently cannot fail; the `Result` keeps the public rendering and
/// writing APIs compatible if future schema validation adds an error.
pub fn render(document: &ExportDocument, format: ExportFormat) -> Result<Vec<u8>, ExportError> {
    let text = match format {
        ExportFormat::Csv => render_csv(document),
        ExportFormat::Json => render_json(document),
    };
    Ok(text.into_bytes())
}

/// Renders and atomically creates an export at an explicit caller path.
///
/// The destination parent must already exist. Existing destinations are never
/// overwritten, including if another process creates one between validation
/// and commit. A temporary sibling is fully synced before a no-clobber atomic
/// link commit, then removed.
///
/// # Errors
///
/// Returns an error for invalid paths, existing targets, or filesystem errors.
pub fn write_export(
    destination: &Path,
    document: &ExportDocument,
    format: ExportFormat,
) -> Result<(), ExportError> {
    let contents = render(document, format)?;
    write_atomic(destination, &contents)
}

/// Atomically creates a byte stream at an explicit, previously non-existent path.
///
/// This lower-level operation is public for command adapters that stream a
/// separately generated schema, but it still applies the same no-overwrite
/// and explicit-path safeguards.
///
/// # Errors
///
/// Returns an error for invalid paths, existing targets, or filesystem errors.
pub fn write_atomic(destination: &Path, contents: &[u8]) -> Result<(), ExportError> {
    let parent = validate_destination(destination)?;
    if destination.exists() {
        return Err(ExportError::DestinationExists(destination.to_path_buf()));
    }

    let temporary = temporary_path(parent, destination);
    let result = (|| -> Result<(), ExportError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);

        // `hard_link` has no replacement mode: if a concurrent writer wins,
        // it fails instead of silently replacing that writer's file. Because
        // both names are in the same directory, the commit is atomic.
        match fs::hard_link(&temporary, destination) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(ExportError::DestinationExists(destination.to_path_buf()))
            }
            Err(error) => Err(ExportError::Io(error)),
        }
    })();

    // A successful hard link leaves a second name for the same completed file;
    // an unsuccessful attempt leaves only the temporary file. Either way it is
    // safe and necessary to clean up this sibling name.
    let _ = fs::remove_file(&temporary);
    result
}

fn validate_destination(destination: &Path) -> Result<&Path, ExportError> {
    if destination.as_os_str().is_empty() || destination.file_name().is_none() {
        return Err(ExportError::InvalidPath(
            "a destination filename is required".to_owned(),
        ));
    }
    let parent = destination.parent().ok_or_else(|| {
        ExportError::InvalidPath("a destination parent directory is required".to_owned())
    })?;
    if !parent.is_dir() {
        return Err(ExportError::InvalidPath(format!(
            "parent directory does not exist or is not a directory: {}",
            parent.display()
        )));
    }
    Ok(parent)
}

static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temporary_path(parent: &Path, destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("export");
    let counter = TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        counter
    ))
}

fn render_csv(document: &ExportDocument) -> String {
    match &document.records {
        ExportRecords::RawSamples(samples) => csv_raw_samples(&document.metadata, samples),
        ExportRecords::Sessions(sessions) => csv_sessions(&document.metadata, sessions),
        ExportRecords::Summaries(summaries) => csv_summaries(&document.metadata, summaries),
    }
}

fn csv_raw_samples(metadata: &ExportMetadata, samples: &[HistorySample]) -> String {
    let mut output = String::from(
        "schema_version,generated_at,timezone,record_type,battery_id,recorded_at,state,percentage,percentage_source,energy_now_wh,energy_now_wh_source,energy_full_wh,energy_full_wh_source,energy_design_wh,energy_design_wh_source,power_watts,power_watts_source,voltage_volts,voltage_volts_source,current_amps,current_amps_source,temperature_celsius,temperature_celsius_source,time_remaining_minutes,time_remaining_minutes_source,cycle_count,cycle_count_source\r\n",
    );
    for sample in samples {
        let m = &sample.metrics;
        csv_row(
            &mut output,
            &[
                EXPORT_SCHEMA_VERSION.to_string(),
                metadata.generated_at.clone(),
                metadata.timezone.clone(),
                "raw_sample".to_owned(),
                sample.battery_id.clone(),
                sample.recorded_at.clone(),
                state_name(sample.state).to_owned(),
                csv_number(m.percentage.value),
                metric_source_name(m.percentage).to_owned(),
                csv_number(m.energy_now_wh.value),
                metric_source_name(m.energy_now_wh).to_owned(),
                csv_number(m.energy_full_wh.value),
                metric_source_name(m.energy_full_wh).to_owned(),
                csv_number(m.energy_design_wh.value),
                metric_source_name(m.energy_design_wh).to_owned(),
                csv_number(m.power_watts.value),
                metric_source_name(m.power_watts).to_owned(),
                csv_number(m.voltage_volts.value),
                metric_source_name(m.voltage_volts).to_owned(),
                csv_number(m.current_amps.value),
                metric_source_name(m.current_amps).to_owned(),
                csv_number(m.temperature_celsius.value),
                metric_source_name(m.temperature_celsius).to_owned(),
                csv_number(m.time_remaining_minutes.value),
                metric_source_name(m.time_remaining_minutes).to_owned(),
                csv_number(m.cycle_count.value),
                metric_source_name(m.cycle_count).to_owned(),
            ],
        );
    }
    output
}

fn csv_sessions(metadata: &ExportMetadata, sessions: &[BatterySession]) -> String {
    let mut output = String::from(
        "schema_version,generated_at,timezone,record_type,battery_id,kind,started_at,ended_at,sample_count,observed_duration_seconds,start_percentage,end_percentage,start_energy_wh,end_energy_wh,average_power_watts,complete,interrupt_reason\r\n",
    );
    for session in sessions {
        csv_row(
            &mut output,
            &[
                EXPORT_SCHEMA_VERSION.to_string(),
                metadata.generated_at.clone(),
                metadata.timezone.clone(),
                "session".to_owned(),
                session.battery_id.clone(),
                session_kind_name(session).to_owned(),
                session.started_at.clone(),
                session.ended_at.clone(),
                session.sample_count.to_string(),
                csv_number(session.observed_duration_seconds),
                csv_number(session.start_percentage),
                csv_number(session.end_percentage),
                csv_number(session.start_energy_wh),
                csv_number(session.end_energy_wh),
                csv_number(session.average_power_watts),
                session.complete.to_string(),
                interrupt_reason_name(session).to_owned(),
            ],
        );
    }
    output
}

fn csv_summaries(metadata: &ExportMetadata, summaries: &[SessionAggregation]) -> String {
    let mut output = String::from(
        "schema_version,generated_at,timezone,record_type,bucket,battery_id,session_count,complete_session_count,observed_duration_seconds\r\n",
    );
    for summary in summaries {
        csv_row(
            &mut output,
            &[
                EXPORT_SCHEMA_VERSION.to_string(),
                metadata.generated_at.clone(),
                metadata.timezone.clone(),
                "summary".to_owned(),
                summary.bucket.clone(),
                summary.battery_id.clone(),
                summary.session_count.to_string(),
                summary.complete_session_count.to_string(),
                csv_number(summary.observed_duration_seconds),
            ],
        );
    }
    output
}

fn csv_row(output: &mut String, fields: &[String]) {
    for (index, field) in fields.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        csv_field(output, field);
    }
    output.push_str("\r\n");
}

fn csv_field(output: &mut String, field: &str) {
    if field.contains([',', '"', '\r', '\n']) {
        output.push('"');
        output.push_str(&field.replace('"', "\"\""));
        output.push('"');
    } else {
        output.push_str(field);
    }
}

fn render_json(document: &ExportDocument) -> String {
    let mut output = format!(
        "{{\"schemaVersion\":{EXPORT_SCHEMA_VERSION},\"generatedAt\":{},\"timezone\":{},\"units\":{{\"percentage\":\"percent\",\"energy\":\"Wh\",\"power\":\"W\",\"voltage\":\"V\",\"current\":\"A\",\"temperature\":\"C\",\"timeRemaining\":\"minutes\"}},",
        json_string(&document.metadata.generated_at),
        json_string(&document.metadata.timezone)
    );
    match &document.records {
        ExportRecords::RawSamples(samples) => {
            output.push_str("\"recordType\":\"rawSamples\",\"records\":[");
            for (i, s) in samples.iter().enumerate() {
                if i != 0 {
                    output.push(',');
                }
                json_raw_sample(&mut output, s);
            }
        }
        ExportRecords::Sessions(sessions) => {
            output.push_str("\"recordType\":\"sessions\",\"records\":[");
            for (i, s) in sessions.iter().enumerate() {
                if i != 0 {
                    output.push(',');
                }
                json_session(&mut output, s);
            }
        }
        ExportRecords::Summaries(summaries) => {
            output.push_str("\"recordType\":\"summaries\",\"records\":[");
            for (i, s) in summaries.iter().enumerate() {
                if i != 0 {
                    output.push(',');
                }
                json_summary(&mut output, s);
            }
        }
    }
    output.push_str("]}");
    output
}

fn json_raw_sample(output: &mut String, sample: &HistorySample) {
    write!(
        output,
        "{{\"batteryId\":{},\"recordedAt\":{},\"state\":{},\"metrics\":{{",
        json_string(&sample.battery_id),
        json_string(&sample.recorded_at),
        json_string(state_name(sample.state))
    )
    .expect("writing to String cannot fail");
    let metrics = [
        ("percentage", sample.metrics.percentage),
        ("energyNowWh", sample.metrics.energy_now_wh),
        ("energyFullWh", sample.metrics.energy_full_wh),
        ("energyDesignWh", sample.metrics.energy_design_wh),
        ("powerWatts", sample.metrics.power_watts),
        ("voltageVolts", sample.metrics.voltage_volts),
        ("currentAmps", sample.metrics.current_amps),
        ("temperatureCelsius", sample.metrics.temperature_celsius),
        (
            "timeRemainingMinutes",
            sample.metrics.time_remaining_minutes,
        ),
        ("cycleCount", sample.metrics.cycle_count),
    ];
    for (index, (name, metric)) in metrics.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write!(
            output,
            "{}:{{\"value\":{},\"source\":{},\"availability\":{},\"freshness\":{}}}",
            json_string(name),
            json_number(metric.value),
            json_string(metric_source_name(*metric)),
            json_string(availability_name(*metric)),
            json_string(freshness_name(*metric))
        )
        .expect("writing to String cannot fail");
    }
    output.push_str("}}");
}

fn json_session(output: &mut String, session: &BatterySession) {
    write!(output, "{{\"batteryId\":{},\"kind\":{},\"startedAt\":{},\"endedAt\":{},\"sampleCount\":{},\"observedDurationSeconds\":{},\"startPercentage\":{},\"endPercentage\":{},\"startEnergyWh\":{},\"endEnergyWh\":{},\"averagePowerWatts\":{},\"complete\":{},\"interruptReason\":{}}}", json_string(&session.battery_id), json_string(session_kind_name(session)), json_string(&session.started_at), json_string(&session.ended_at), session.sample_count, json_number(session.observed_duration_seconds), json_number(session.start_percentage), json_number(session.end_percentage), json_number(session.start_energy_wh), json_number(session.end_energy_wh), json_number(session.average_power_watts), session.complete, json_string(interrupt_reason_name(session))).expect("writing to String cannot fail");
}

fn json_summary(output: &mut String, summary: &SessionAggregation) {
    write!(output, "{{\"bucket\":{},\"batteryId\":{},\"sessionCount\":{},\"completeSessionCount\":{},\"observedDurationSeconds\":{}}}", json_string(&summary.bucket), json_string(&summary.battery_id), summary.session_count, summary.complete_session_count, json_number(summary.observed_duration_seconds)).expect("writing to String cannot fail");
}

fn json_string(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() + 2);
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\u{08}' => encoded.push_str("\\b"),
            '\u{0C}' => encoded.push_str("\\f"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            control if control.is_control() => {
                write!(encoded, "\\u{:04x}", u32::from(control))
                    .expect("writing to String cannot fail");
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    encoded
}
fn json_number(value: Option<f64>) -> String {
    value
        .filter(|number| number.is_finite())
        .map_or_else(|| "null".to_owned(), |number| number.to_string())
}
fn csv_number(value: Option<f64>) -> String {
    value
        .filter(|number| number.is_finite())
        .map_or_else(String::new, |number| number.to_string())
}

fn state_name(state: crate::storage::SampleState) -> &'static str {
    match state {
        crate::storage::SampleState::Charging => "charging",
        crate::storage::SampleState::Discharging => "discharging",
        crate::storage::SampleState::Full => "full",
        crate::storage::SampleState::Idle => "idle",
        crate::storage::SampleState::Unknown => "unknown",
    }
}
fn metric_source_name(metric: HistoryMetric) -> &'static str {
    match metric.source {
        crate::storage::MetricSource::Upower => "upower",
        crate::storage::MetricSource::Sysfs => "sysfs",
        crate::storage::MetricSource::Derived => "derived",
        crate::storage::MetricSource::Unavailable => "unavailable",
    }
}
fn availability_name(metric: HistoryMetric) -> &'static str {
    match metric.availability {
        crate::storage::HistoryAvailability::Available => "available",
        crate::storage::HistoryAvailability::Unavailable => "unavailable",
    }
}
fn freshness_name(metric: HistoryMetric) -> &'static str {
    match metric.freshness {
        crate::storage::HistoryFreshness::Recorded => "recorded",
        crate::storage::HistoryFreshness::Unavailable => "unavailable",
    }
}
fn session_kind_name(session: &BatterySession) -> &'static str {
    match session.kind {
        crate::storage::BatterySessionKind::Charging => "charging",
        crate::storage::BatterySessionKind::Discharging => "discharging",
        crate::storage::BatterySessionKind::Full => "full",
        crate::storage::BatterySessionKind::Unknown => "unknown",
    }
}
fn interrupt_reason_name(session: &BatterySession) -> &'static str {
    match session.interrupt_reason {
        crate::storage::SessionInterruptReason::StateChanged => "state_changed",
        crate::storage::SessionInterruptReason::BootChanged => "boot_changed",
        crate::storage::SessionInterruptReason::SampleGap => "sample_gap",
        crate::storage::SessionInterruptReason::DataEnded => "data_ended",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{
        HistoryAvailability, HistoryFreshness, HistoryMetrics, MetricSource, SampleState,
    };

    fn metric(value: Option<f64>) -> HistoryMetric {
        HistoryMetric {
            value,
            source: value.map_or(MetricSource::Unavailable, |_| MetricSource::Upower),
            availability: value.map_or(HistoryAvailability::Unavailable, |_| {
                HistoryAvailability::Available
            }),
            freshness: value.map_or(HistoryFreshness::Unavailable, |_| {
                HistoryFreshness::Recorded
            }),
        }
    }
    fn sample() -> HistorySample {
        HistorySample {
            battery_id: "BAT,\"0".to_owned(),
            recorded_at: "2026-08-23T10:00:00Z".to_owned(),
            boot_id: "not-exported".to_owned(),
            boot_seconds: 1.0,
            state: SampleState::Charging,
            metrics: HistoryMetrics {
                percentage: metric(Some(55.5)),
                energy_now_wh: metric(None),
                energy_full_wh: metric(None),
                energy_design_wh: metric(None),
                power_watts: metric(None),
                voltage_volts: metric(None),
                current_amps: metric(None),
                temperature_celsius: metric(None),
                time_remaining_minutes: metric(None),
                cycle_count: metric(None),
            },
        }
    }
    fn document() -> ExportDocument {
        ExportDocument {
            metadata: ExportMetadata {
                generated_at: "2026-08-23T10:00:01Z".to_owned(),
                timezone: "Europe/Rome".to_owned(),
            },
            records: ExportRecords::RawSamples(vec![sample()]),
        }
    }

    #[test]
    fn csv_uses_stable_header_escapes_values_and_keeps_nulls_empty() {
        let csv = String::from_utf8(render(&document(), ExportFormat::Csv).unwrap()).unwrap();
        assert!(csv.starts_with("schema_version,generated_at,timezone,record_type,battery_id,"));
        assert!(csv.contains("\"BAT,\"\"0\""));
        assert!(csv.contains(",55.5,upower,,unavailable,"));
        assert!(!csv.contains("not-exported"));
    }

    #[test]
    fn json_schema_preserves_typed_nulls_and_has_no_serial_or_boot_identifier() {
        let json = String::from_utf8(render(&document(), ExportFormat::Json).unwrap()).unwrap();
        assert!(json.starts_with("{\"schemaVersion\":1,"));
        assert!(json.contains("\"value\":55.5"));
        assert!(json.contains("\"energyNowWh\":{\"value\":null,"));
        assert!(!json.contains("bootId"));
        assert!(!json.contains("not-exported"));
        // A lightweight round-trip property: JSON escaping is reversible for
        // the only user-controlled string in this fixture.
        assert!(json.contains("BAT,\\\"0"));
    }

    #[test]
    fn atomic_write_creates_once_and_refuses_existing_destination() {
        let directory = std::env::temp_dir().join(format!(
            "battery-dashboard-export-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("history.csv");
        write_atomic(&destination, b"first").unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"first");
        assert!(matches!(
            write_atomic(&destination, b"second"),
            Err(ExportError::DestinationExists(_))
        ));
        assert_eq!(fs::read(&destination).unwrap(), b"first");
        fs::remove_file(&destination).unwrap();
        fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn atomic_write_rejects_missing_parent_without_creating_a_file() {
        let destination = std::env::temp_dir()
            .join("battery-dashboard-no-such-export-parent")
            .join("history.csv");
        assert!(matches!(
            write_atomic(&destination, b"data"),
            Err(ExportError::InvalidPath(_))
        ));
        assert!(!destination.exists());
    }
}
