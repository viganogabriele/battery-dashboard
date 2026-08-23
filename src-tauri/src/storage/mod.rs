//! Local SQLite persistence for immutable battery telemetry samples.
//!
//! The recorder supplies a UTC instant and Linux boot-relative identity; this
//! module does not manufacture readings, timestamps, or source provenance.

use std::{
    env,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, Row, TransactionBehavior, params,
};
use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const APPLICATION_DIRECTORY: &str = "battery-dashboard";
const DATABASE_FILE: &str = "battery.sqlite3";
const BUSY_TIMEOUT_MILLISECONDS: u32 = 5_000;
/// The recorder normally writes once a minute. A longer interval is an observed
/// discontinuity (for example suspend, shutdown, or a stopped recorder), not a
/// line that a chart may safely interpolate.
const MAX_CONTIGUOUS_SAMPLE_SECONDS: f64 = 180.0;

const MIGRATIONS: &[&str] = &[include_str!("../../migrations/0001_initial.sql")];

/// An error raised while locating, migrating, validating, or writing the local database.
#[derive(Debug)]
pub enum StorageError {
    /// The XDG data directory cannot be determined without guessing a path.
    DataDirectoryUnavailable,
    /// A caller supplied an invalid sample.
    InvalidSample(String),
    /// A caller requested a history interval that cannot be interpreted safely.
    InvalidHistoryQuery(String),
    /// The database is newer than this version of the application understands.
    UnsupportedSchemaVersion(i64),
    /// An operating-system filesystem operation failed.
    Io(std::io::Error),
    /// `SQLite` rejected an operation.
    Sqlite(rusqlite::Error),
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DataDirectoryUnavailable => formatter.write_str(
                "could not resolve XDG_DATA_HOME; set XDG_DATA_HOME or HOME before starting the recorder",
            ),
            Self::InvalidSample(message) => write!(formatter, "invalid battery sample: {message}"),
            Self::InvalidHistoryQuery(message) => write!(formatter, "invalid history query: {message}"),
            Self::UnsupportedSchemaVersion(version) => {
                write!(formatter, "database schema version {version} is newer than supported")
            }
            Self::Io(error) => error.fmt(formatter),
            Self::Sqlite(error) => error.fmt(formatter),
        }
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            Self::DataDirectoryUnavailable
            | Self::InvalidSample(_)
            | Self::InvalidHistoryQuery(_)
            | Self::UnsupportedSchemaVersion(_) => None,
        }
    }
}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// The origin of a metric value as recorded by a Linux battery provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricSource {
    /// Read from `UPower` over the local system D-Bus.
    Upower,
    /// Read directly from Linux sysfs.
    Sysfs,
    /// Calculated only from compatible observed fields.
    Derived,
    /// No provider supplied a usable value.
    Unavailable,
}

impl MetricSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Upower => "upower",
            Self::Sysfs => "sysfs",
            Self::Derived => "derived",
            Self::Unavailable => "unavailable",
        }
    }
}

/// An optional metric and the provider that supplied it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampleMetric {
    /// The observed value, if a provider supplied one.
    pub value: Option<f64>,
    /// The value provenance, or [`MetricSource::Unavailable`] when absent.
    pub source: MetricSource,
}

impl SampleMetric {
    /// Creates an unavailable metric without substituting a value.
    #[must_use]
    pub const fn unavailable() -> Self {
        Self {
            value: None,
            source: MetricSource::Unavailable,
        }
    }
}

/// The state of one battery at the time a sample was collected.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SampleState {
    /// The battery is receiving energy.
    Charging,
    /// The battery is providing energy.
    Discharging,
    /// The battery is reported full.
    Full,
    /// The battery is neither charging nor discharging.
    Idle,
    /// The provider did not report a usable state.
    Unknown,
}

impl SampleState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Charging => "charging",
            Self::Discharging => "discharging",
            Self::Full => "full",
            Self::Idle => "idle",
            Self::Unknown => "unknown",
        }
    }
}

/// The metrics stored for one immutable battery observation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampleMetrics {
    /// Charge level, expressed as a percentage.
    pub percentage: SampleMetric,
    /// Current stored energy in watt-hours.
    pub energy_now_wh: SampleMetric,
    /// Current maximum energy capacity in watt-hours.
    pub energy_full_wh: SampleMetric,
    /// Design energy capacity in watt-hours.
    pub energy_design_wh: SampleMetric,
    /// Instantaneous signed power in watts.
    pub power_watts: SampleMetric,
    /// Battery voltage in volts.
    pub voltage_volts: SampleMetric,
    /// Signed current in amperes.
    pub current_amps: SampleMetric,
    /// Battery temperature in degrees Celsius.
    pub temperature_celsius: SampleMetric,
    /// Provider-estimated remaining minutes, if available.
    pub time_remaining_minutes: SampleMetric,
    /// Reported battery cycle count.
    pub cycle_count: SampleMetric,
}

/// Input needed to store exactly one physical battery observation.
#[derive(Clone, Debug, PartialEq)]
pub struct NewBatterySample {
    /// Stable physical battery identifier, such as `BAT0`.
    pub battery_id: String,
    /// UTC collection time. It is serialized as RFC 3339 with an explicit offset.
    pub recorded_at: OffsetDateTime,
    /// Linux boot ID from `/proc/sys/kernel/random/boot_id`.
    pub boot_id: String,
    /// Monotonic seconds since the boot identified by [`Self::boot_id`].
    pub boot_seconds: f64,
    /// Charging state reported at collection time.
    pub state: SampleState,
    /// Field-level values and provenance.
    pub metrics: SampleMetrics,
}

/// The outcome of trying to write one idempotent sample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertOutcome {
    /// A new immutable row was created.
    Inserted,
    /// A sample with the same timestamp or boot-relative identity already exists.
    Duplicate,
}

/// A bounded, UTC history read. `battery_id` filters one physical battery;
/// callers that need an aggregate must combine compatible batteries explicitly.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryQuery {
    /// Inclusive UTC range start.
    pub start: OffsetDateTime,
    /// Inclusive UTC range end.
    pub end: OffsetDateTime,
    /// Optional physical battery identifier.
    pub battery_id: Option<String>,
    /// Preferred upper bound for returned sample observations.
    ///
    /// The reader may retain additional anchor observations when necessary to
    /// express a real discontinuity rather than silently joining it.
    pub max_points: usize,
}

/// The durable availability of a historical metric.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryAvailability {
    /// A finite provider value was recorded.
    Available,
    /// No provider value was recorded; this is not a zero.
    Unavailable,
}

/// Freshness at the time the recorder committed an immutable observation.
/// Historical reads never relabel an old value as current or synthesize a
/// `stale` provider result: consumers can use `recorded_at` to judge age.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryFreshness {
    /// The provider value was observed during this recorder run.
    Recorded,
    /// No provider value existed for this recorder run.
    Unavailable,
}

/// One historical metric with its original field-level provenance.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMetric {
    /// Recorded value, when available.
    pub value: Option<f64>,
    /// Provider that supplied the recorded value.
    pub source: MetricSource,
    /// Explicitly distinguishes absent telemetry from a numerical zero.
    pub availability: HistoryAvailability,
    /// Whether this exact historical sample recorded a provider value.
    pub freshness: HistoryFreshness,
}

/// Metrics retained for one immutable historical observation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct HistoryMetrics {
    pub percentage: HistoryMetric,
    pub energy_now_wh: HistoryMetric,
    pub energy_full_wh: HistoryMetric,
    pub energy_design_wh: HistoryMetric,
    pub power_watts: HistoryMetric,
    pub voltage_volts: HistoryMetric,
    pub current_amps: HistoryMetric,
    pub temperature_celsius: HistoryMetric,
    pub time_remaining_minutes: HistoryMetric,
    pub cycle_count: HistoryMetric,
}

/// One immutable telemetry sample exposed to the desktop frontend.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct HistorySample {
    /// Stable physical battery identifier, such as `BAT0`. Aggregate views are
    /// intentionally composed by a higher layer from compatible batteries.
    pub battery_id: String,
    /// UTC RFC 3339 timestamp, preserved exactly as a wall-clock observation.
    pub recorded_at: String,
    /// Linux boot identity captured by the recorder.
    pub boot_id: String,
    /// Monotonic seconds from the identified boot.
    pub boot_seconds: f64,
    pub state: SampleState,
    pub metrics: HistoryMetrics,
}

/// Why a chart must not interpolate between two observations.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryGapReason {
    /// Linux boot identity changed, so monotonic timing is discontinuous.
    BootChanged,
    /// A sample interval exceeded the recorder's continuity limit.
    SampleIntervalExceeded,
}

/// A real gap retained in the timeline even when samples are downsampled.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct HistoryGap {
    /// Timestamp of the last sample before the discontinuity.
    pub from: String,
    /// Timestamp of the first sample after the discontinuity.
    pub to: String,
    pub reason: HistoryGapReason,
}

/// A timeline item for charting. Consumers must break a line at every gap.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[allow(missing_docs)]
pub enum HistoryTimelineItem {
    Sample(Box<HistorySample>),
    Gap(HistoryGap),
}

/// Statistics for one metric over the raw, undiscarded query result.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct HistoryMetricSummary {
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub average: Option<f64>,
}

/// Summary calculated before visual downsampling.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct HistorySummary {
    /// Number of raw, durable samples in the requested range.
    pub sample_count: usize,
    /// Total duration across contiguous observed intervals only.
    pub observed_duration_seconds: Option<f64>,
    /// Signed watt-hours integrated from power only when every observed
    /// contiguous interval contains a valid power reading. Otherwise absent.
    pub observed_energy_wh: Option<f64>,
    pub percentage: HistoryMetricSummary,
    pub energy_now_wh: HistoryMetricSummary,
    pub power_watts: HistoryMetricSummary,
    pub voltage_volts: HistoryMetricSummary,
    pub current_amps: HistoryMetricSummary,
    pub temperature_celsius: HistoryMetricSummary,
}

/// Complete response for a bounded historical read.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct HistoryResponse {
    /// The query bounds rendered in UTC RFC 3339 for frontend diagnostics.
    pub start: String,
    pub end: String,
    pub battery_id: Option<String>,
    /// Timeline sample and gap markers after deterministic downsampling.
    pub timeline: Vec<HistoryTimelineItem>,
    pub summary: HistorySummary,
}

/// A migrated connection to Battery Dashboard's local `SQLite` history database.
pub struct Storage {
    connection: Connection,
}

impl Storage {
    /// Opens the database under the current user's XDG data directory.
    ///
    /// The resolved path is `$XDG_DATA_HOME/battery-dashboard/battery.sqlite3`,
    /// or `$HOME/.local/share/battery-dashboard/battery.sqlite3` when the XDG
    /// variable is not set. No global or system directory is used.
    ///
    /// # Errors
    ///
    /// Returns an error when no user data directory can be resolved, the path
    /// cannot be created, or `SQLite` cannot open or migrate the database.
    pub fn open_default() -> Result<Self, StorageError> {
        Self::open_at(default_database_path()?)
    }

    /// Opens and migrates a database at an explicit path.
    ///
    /// This injection point is intended for the recorder and deterministic tests.
    ///
    /// # Errors
    ///
    /// Returns an error when the parent directory cannot be created, `SQLite`
    /// cannot open or configure the file, or a migration fails.
    pub fn open_at(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        configure_connection(&connection)?;
        migrate(&connection)?;

        Ok(Self { connection })
    }

    /// Inserts an immutable sample, or returns [`InsertOutcome::Duplicate`] if
    /// the same battery was already recorded at the supplied UTC or boot-relative instant.
    ///
    /// # Errors
    ///
    /// Returns an error when the sample is invalid or `SQLite` cannot begin,
    /// write, or commit the transaction.
    pub fn insert_sample(
        &mut self,
        sample: &NewBatterySample,
    ) -> Result<InsertOutcome, StorageError> {
        validate_sample(sample)?;
        let recorded_at = sample
            .recorded_at
            .format(&Rfc3339)
            .map_err(|error| StorageError::InvalidSample(error.to_string()))?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = transaction.execute(
            "INSERT INTO battery_samples (
                battery_id, recorded_at_utc, boot_id, boot_seconds, state,
                percentage, percentage_source, energy_now_wh, energy_now_wh_source,
                energy_full_wh, energy_full_wh_source, energy_design_wh, energy_design_wh_source,
                power_watts, power_watts_source, voltage_volts, voltage_volts_source,
                current_amps, current_amps_source, temperature_celsius, temperature_celsius_source,
                time_remaining_minutes, time_remaining_minutes_source, cycle_count, cycle_count_source
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25
            )",
            params![
                sample.battery_id,
                recorded_at,
                sample.boot_id,
                sample.boot_seconds,
                sample.state.as_str(),
                sample.metrics.percentage.value,
                sample.metrics.percentage.source.as_str(),
                sample.metrics.energy_now_wh.value,
                sample.metrics.energy_now_wh.source.as_str(),
                sample.metrics.energy_full_wh.value,
                sample.metrics.energy_full_wh.source.as_str(),
                sample.metrics.energy_design_wh.value,
                sample.metrics.energy_design_wh.source.as_str(),
                sample.metrics.power_watts.value,
                sample.metrics.power_watts.source.as_str(),
                sample.metrics.voltage_volts.value,
                sample.metrics.voltage_volts.source.as_str(),
                sample.metrics.current_amps.value,
                sample.metrics.current_amps.source.as_str(),
                sample.metrics.temperature_celsius.value,
                sample.metrics.temperature_celsius.source.as_str(),
                sample.metrics.time_remaining_minutes.value,
                sample.metrics.time_remaining_minutes.source.as_str(),
                sample.metrics.cycle_count.value,
                sample.metrics.cycle_count.source.as_str(),
            ],
        );

        match result {
            Ok(_) => {
                transaction.commit()?;
                Ok(InsertOutcome::Inserted)
            }
            Err(error) if is_unique_constraint(&error) => Ok(InsertOutcome::Duplicate),
            Err(error) => Err(error.into()),
        }
    }

    /// Returns the number of stored samples. Intended for diagnostics and tests.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot read the sample table.
    pub fn sample_count(&self) -> Result<u64, StorageError> {
        let count =
            self.connection
                .query_row("SELECT COUNT(*) FROM battery_samples", [], |row| {
                    row.get::<_, u64>(0)
                })?;
        Ok(count)
    }

    /// Verifies `SQLite`'s internal consistency check for the current database.
    ///
    /// # Errors
    ///
    /// Returns an error if `SQLite` cannot run the check or reports a result
    /// other than `ok`.
    pub fn integrity_check(&self) -> Result<(), StorageError> {
        let result: String = self
            .connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if result == "ok" {
            Ok(())
        } else {
            Err(StorageError::InvalidSample(format!(
                "SQLite integrity check failed: {result}"
            )))
        }
    }

    /// Returns the schema version after all migrations have been applied.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot read the version or it cannot fit
    /// in an unsigned schema version.
    pub fn schema_version(&self) -> Result<u32, StorageError> {
        let version: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        u32::try_from(version).map_err(|_| StorageError::UnsupportedSchemaVersion(version))
    }

    /// Returns the most recent sample time, without treating a missing sample as a reading.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot read the table or a stored timestamp
    /// is not valid RFC 3339.
    pub fn last_recorded_at(&self) -> Result<Option<OffsetDateTime>, StorageError> {
        let timestamp: Option<String> = self
            .connection
            .query_row(
                "SELECT recorded_at_utc FROM battery_samples ORDER BY recorded_at_utc DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        timestamp
            .map(|value| {
                OffsetDateTime::parse(&value, &Rfc3339).map_err(|error| {
                    StorageError::InvalidSample(format!(
                        "database contains an invalid UTC timestamp {value:?}: {error}"
                    ))
                })
            })
            .transpose()
    }

    /// Reads a bounded history from this already-open database.
    ///
    /// The response keeps provider provenance and records real gaps caused by a
    /// reboot or a sufficiently long recorder interruption. It never fills a
    /// missing metric or interpolates an unobserved interval.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid range, unreadable rows, or malformed
    /// durable data.
    pub fn history(&self, query: &HistoryQuery) -> Result<HistoryResponse, StorageError> {
        validate_history_query(query)?;
        let start = format_utc(query.start)?;
        let end = format_utc(query.end)?;
        let mut statement = self.connection.prepare(
            "SELECT battery_id, recorded_at_utc, boot_id, boot_seconds, state,
                    percentage, percentage_source, energy_now_wh, energy_now_wh_source,
                    energy_full_wh, energy_full_wh_source, energy_design_wh, energy_design_wh_source,
                    power_watts, power_watts_source, voltage_volts, voltage_volts_source,
                    current_amps, current_amps_source, temperature_celsius, temperature_celsius_source,
                    time_remaining_minutes, time_remaining_minutes_source, cycle_count, cycle_count_source
             FROM battery_samples
             WHERE recorded_at_utc >= ?1 AND recorded_at_utc <= ?2
               AND (?3 IS NULL OR battery_id = ?3)
             ORDER BY recorded_at_utc ASC, id ASC",
        )?;
        let raw = statement
            .query_map(
                params![start, end, query.battery_id.as_deref()],
                raw_history_sample,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        history_response(query, &raw)
    }

    #[cfg(test)]
    fn connection(&self) -> &Connection {
        &self.connection
    }
}

/// Reads history from the default database only if it already exists.
///
/// This does not create the XDG directory or an empty database. A missing
/// database returns `Ok(None)`, allowing a caller to present an honest empty
/// state before recording has ever been enabled.
///
/// # Errors
///
/// Returns an error when a present database cannot be read or its data is not
/// a valid immutable telemetry record.
pub fn history_if_exists(query: &HistoryQuery) -> Result<Option<HistoryResponse>, StorageError> {
    let Some(path) = existing_database_path()? else {
        return Ok(None);
    };
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    configure_read_connection(&connection)?;
    let storage = Storage { connection };
    storage.history(query).map(Some)
}

/// Resolves a database path below an explicitly supplied XDG data home.
#[must_use]
pub fn database_path_from_data_home(data_home: impl AsRef<Path>) -> PathBuf {
    data_home
        .as_ref()
        .join(APPLICATION_DIRECTORY)
        .join(DATABASE_FILE)
}

/// Resolves the current user's XDG data location without creating it.
///
/// # Errors
///
/// Returns [`StorageError::DataDirectoryUnavailable`] when neither
/// `XDG_DATA_HOME` nor `HOME` supplies a non-empty path.
pub fn default_database_path() -> Result<PathBuf, StorageError> {
    if let Some(data_home) = env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        return Ok(database_path_from_data_home(PathBuf::from(data_home)));
    }

    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(|home| database_path_from_data_home(PathBuf::from(home).join(".local/share")))
        .ok_or(StorageError::DataDirectoryUnavailable)
}

/// Returns the database path only when a regular database file already exists.
///
/// This performs no directory creation and is appropriate for status checks
/// while recording has not been enabled yet.
///
/// # Errors
///
/// Returns an error only when the XDG data location itself cannot be resolved.
pub fn existing_database_path() -> Result<Option<PathBuf>, StorageError> {
    let path = default_database_path()?;
    Ok(path.is_file().then_some(path))
}

/// Reads the newest collection time from an existing database without creating one.
///
/// A missing database returns `Ok(None)`; it is not evidence of a missing or
/// empty battery reading. The connection is opened read-only.
///
/// # Errors
///
/// Returns an error when the existing database cannot be read or contains an
/// invalid timestamp.
pub fn last_recorded_at_if_exists() -> Result<Option<OffsetDateTime>, StorageError> {
    let Some(path) = existing_database_path()? else {
        return Ok(None);
    };
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let timestamp: Option<String> = connection
        .query_row(
            "SELECT recorded_at_utc FROM battery_samples ORDER BY recorded_at_utc DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    timestamp
        .map(|value| {
            OffsetDateTime::parse(&value, &Rfc3339).map_err(|error| {
                StorageError::InvalidSample(format!(
                    "database contains an invalid UTC timestamp {value:?}: {error}"
                ))
            })
        })
        .transpose()
}

#[derive(Clone, Debug)]
struct RawHistorySample {
    battery_id: String,
    recorded_at: OffsetDateTime,
    sample: HistorySample,
}

fn raw_history_sample(row: &Row<'_>) -> rusqlite::Result<RawHistorySample> {
    let recorded_at: String = row.get(1)?;
    let timestamp = OffsetDateTime::parse(&recorded_at, &Rfc3339).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(RawHistorySample {
        battery_id: row.get(0)?,
        recorded_at: timestamp.to_offset(time::UtcOffset::UTC),
        sample: HistorySample {
            battery_id: row.get(0)?,
            recorded_at,
            boot_id: row.get(2)?,
            boot_seconds: row.get(3)?,
            state: sample_state_from_database(&row.get::<_, String>(4)?)?,
            metrics: HistoryMetrics {
                percentage: history_metric(row, 5, 6)?,
                energy_now_wh: history_metric(row, 7, 8)?,
                energy_full_wh: history_metric(row, 9, 10)?,
                energy_design_wh: history_metric(row, 11, 12)?,
                power_watts: history_metric(row, 13, 14)?,
                voltage_volts: history_metric(row, 15, 16)?,
                current_amps: history_metric(row, 17, 18)?,
                temperature_celsius: history_metric(row, 19, 20)?,
                time_remaining_minutes: history_metric(row, 21, 22)?,
                cycle_count: history_metric(row, 23, 24)?,
            },
        },
    })
}

fn history_metric(
    row: &Row<'_>,
    value_index: usize,
    source_index: usize,
) -> rusqlite::Result<HistoryMetric> {
    let value: Option<f64> = row.get(value_index)?;
    let source = metric_source_from_database(&row.get::<_, String>(source_index)?)?;
    let availability = if value.is_some() {
        HistoryAvailability::Available
    } else {
        HistoryAvailability::Unavailable
    };
    Ok(HistoryMetric {
        value,
        source,
        availability,
        freshness: if value.is_some() {
            HistoryFreshness::Recorded
        } else {
            HistoryFreshness::Unavailable
        },
    })
}

fn metric_source_from_database(value: &str) -> rusqlite::Result<MetricSource> {
    match value {
        "upower" => Ok(MetricSource::Upower),
        "sysfs" => Ok(MetricSource::Sysfs),
        "derived" => Ok(MetricSource::Derived),
        "unavailable" => Ok(MetricSource::Unavailable),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown metric source {value:?}").into(),
        )),
    }
}

fn sample_state_from_database(value: &str) -> rusqlite::Result<SampleState> {
    match value {
        "charging" => Ok(SampleState::Charging),
        "discharging" => Ok(SampleState::Discharging),
        "full" => Ok(SampleState::Full),
        "idle" => Ok(SampleState::Idle),
        "unknown" => Ok(SampleState::Unknown),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown sample state {value:?}").into(),
        )),
    }
}

fn validate_history_query(query: &HistoryQuery) -> Result<(), StorageError> {
    if query.start > query.end {
        return Err(StorageError::InvalidHistoryQuery(
            "start must not be after end".to_owned(),
        ));
    }
    if query.max_points == 0 {
        return Err(StorageError::InvalidHistoryQuery(
            "max_points must be at least one".to_owned(),
        ));
    }
    if query
        .battery_id
        .as_deref()
        .is_some_and(|battery_id| battery_id.trim().is_empty())
    {
        return Err(StorageError::InvalidHistoryQuery(
            "battery_id must not be empty when supplied".to_owned(),
        ));
    }
    Ok(())
}

fn format_utc(timestamp: OffsetDateTime) -> Result<String, StorageError> {
    timestamp
        .to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .map_err(|error| StorageError::InvalidHistoryQuery(error.to_string()))
}

fn history_response(
    query: &HistoryQuery,
    raw: &[RawHistorySample],
) -> Result<HistoryResponse, StorageError> {
    let gaps = history_gaps(raw);
    let summary = history_summary(raw, &gaps);
    let selected = downsample_indices(raw.len(), query.max_points, &gaps);
    let mut timeline = Vec::with_capacity(selected.len() + gaps.len());
    for index in selected {
        timeline.push(HistoryTimelineItem::Sample(Box::new(
            raw[index].sample.clone(),
        )));
        for (_, gap) in gaps.iter().filter(|(gap_index, _)| *gap_index == index) {
            timeline.push(HistoryTimelineItem::Gap(gap.clone()));
        }
    }
    Ok(HistoryResponse {
        start: format_utc(query.start)?,
        end: format_utc(query.end)?,
        battery_id: query.battery_id.clone(),
        timeline,
        summary,
    })
}

fn history_gaps(raw: &[RawHistorySample]) -> Vec<(usize, HistoryGap)> {
    use std::collections::BTreeMap;

    let mut indices_by_battery = BTreeMap::<&str, Vec<usize>>::new();
    for (index, sample) in raw.iter().enumerate() {
        indices_by_battery
            .entry(sample.battery_id.as_str())
            .or_default()
            .push(index);
    }
    let mut gaps = indices_by_battery
        .into_values()
        .flat_map(|indices| {
            indices
                .windows(2)
                .filter_map(|pair| {
                    let first_index = pair[0];
                    let second_index = pair[1];
                    let first = &raw[first_index];
                    let second = &raw[second_index];
                    let reason = if first.sample.boot_id != second.sample.boot_id {
                        Some(HistoryGapReason::BootChanged)
                    } else if (second.recorded_at - first.recorded_at).as_seconds_f64()
                        > MAX_CONTIGUOUS_SAMPLE_SECONDS
                    {
                        Some(HistoryGapReason::SampleIntervalExceeded)
                    } else {
                        None
                    }?;
                    Some((
                        first_index,
                        HistoryGap {
                            from: first.sample.recorded_at.clone(),
                            to: second.sample.recorded_at.clone(),
                            reason,
                        },
                    ))
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    gaps.sort_by_key(|(index, _)| *index);
    gaps
}

fn downsample_indices(
    length: usize,
    max_points: usize,
    gaps: &[(usize, HistoryGap)],
) -> Vec<usize> {
    use std::collections::BTreeSet;

    if length == 0 {
        return Vec::new();
    }
    let mut selected = BTreeSet::from([0, length - 1]);
    for (index, _) in gaps {
        selected.insert(*index);
        selected.insert(index + 1);
    }
    let target_count = max_points.max(selected.len()).min(length);
    if selected.len() < target_count {
        let candidates = target_count.saturating_sub(2);
        for slot in 1..=candidates {
            let numerator = slot * (length - 1);
            let index = (numerator + (candidates / 2)) / (candidates + 1);
            selected.insert(index);
            if selected.len() == target_count {
                break;
            }
        }
        for index in 0..length {
            if selected.len() == target_count {
                break;
            }
            selected.insert(index);
        }
    }
    selected.into_iter().collect()
}

fn history_summary(raw: &[RawHistorySample], gaps: &[(usize, HistoryGap)]) -> HistorySummary {
    let discontinuities = gaps
        .iter()
        .map(|(index, _)| *index)
        .collect::<std::collections::BTreeSet<_>>();
    let mut observed_seconds = 0.0;
    let mut observed_energy_watt_seconds = 0.0;
    let mut energy_supported = raw.len() >= 2;
    for (index, pair) in raw.windows(2).enumerate() {
        if discontinuities.contains(&index) {
            continue;
        }
        let seconds = (pair[1].recorded_at - pair[0].recorded_at).as_seconds_f64();
        if seconds <= 0.0 {
            continue;
        }
        observed_seconds += seconds;
        match (
            pair[0].sample.metrics.power_watts.value,
            pair[1].sample.metrics.power_watts.value,
        ) {
            (Some(first), Some(second)) => {
                observed_energy_watt_seconds += first.midpoint(second) * seconds;
            }
            _ => energy_supported = false,
        }
    }
    HistorySummary {
        sample_count: raw.len(),
        observed_duration_seconds: (observed_seconds > 0.0).then_some(observed_seconds),
        observed_energy_wh: (energy_supported && observed_seconds > 0.0)
            .then_some(observed_energy_watt_seconds / 3600.0),
        percentage: metric_summary(
            raw.iter()
                .map(|sample| sample.sample.metrics.percentage.value),
        ),
        energy_now_wh: metric_summary(
            raw.iter()
                .map(|sample| sample.sample.metrics.energy_now_wh.value),
        ),
        power_watts: metric_summary(
            raw.iter()
                .map(|sample| sample.sample.metrics.power_watts.value),
        ),
        voltage_volts: metric_summary(
            raw.iter()
                .map(|sample| sample.sample.metrics.voltage_volts.value),
        ),
        current_amps: metric_summary(
            raw.iter()
                .map(|sample| sample.sample.metrics.current_amps.value),
        ),
        temperature_celsius: metric_summary(
            raw.iter()
                .map(|sample| sample.sample.metrics.temperature_celsius.value),
        ),
    }
}

fn metric_summary(values: impl Iterator<Item = Option<f64>>) -> HistoryMetricSummary {
    let values = values.flatten().collect::<Vec<_>>();
    let Some(first) = values.first().copied() else {
        return HistoryMetricSummary {
            minimum: None,
            maximum: None,
            average: None,
        };
    };
    let (minimum, maximum, sum) = values
        .iter()
        .copied()
        .fold((first, first, 0.0), |(min, max, total), value| {
            (min.min(value), max.max(value), total + value)
        });
    let count = u32::try_from(values.len()).ok();
    HistoryMetricSummary {
        minimum: Some(minimum),
        maximum: Some(maximum),
        average: count.map(|count| sum / f64::from(count)),
    }
}

fn configure_connection(connection: &Connection) -> Result<(), StorageError> {
    connection.busy_timeout(std::time::Duration::from_millis(u64::from(
        BUSY_TIMEOUT_MILLISECONDS,
    )))?;
    connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    Ok(())
}

fn configure_read_connection(connection: &Connection) -> Result<(), StorageError> {
    connection.busy_timeout(std::time::Duration::from_millis(u64::from(
        BUSY_TIMEOUT_MILLISECONDS,
    )))?;
    Ok(())
}

fn migrate(connection: &Connection) -> Result<(), StorageError> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let known_version = i64::try_from(MIGRATIONS.len()).expect("migration count fits in i64");
    if version > known_version {
        return Err(StorageError::UnsupportedSchemaVersion(version));
    }

    let applied_migrations =
        usize::try_from(version).map_err(|_| StorageError::UnsupportedSchemaVersion(version))?;
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(applied_migrations) {
        let migration_version = i64::try_from(index + 1).expect("migration index fits in i64");
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(migration)?;
        transaction.pragma_update(None, "user_version", migration_version)?;
        transaction.commit()?;
    }
    Ok(())
}

fn validate_sample(sample: &NewBatterySample) -> Result<(), StorageError> {
    if sample.battery_id.trim().is_empty() {
        return Err(StorageError::InvalidSample(
            "battery_id is empty".to_owned(),
        ));
    }
    if sample.boot_id.trim().is_empty() {
        return Err(StorageError::InvalidSample("boot_id is empty".to_owned()));
    }
    if !sample.boot_seconds.is_finite() || sample.boot_seconds < 0.0 {
        return Err(StorageError::InvalidSample(
            "boot_seconds must be finite and non-negative".to_owned(),
        ));
    }

    for (name, metric) in sample_metrics(sample) {
        if metric.value.is_some_and(|value| !value.is_finite()) {
            return Err(StorageError::InvalidSample(format!(
                "{name} must be finite when available"
            )));
        }
        match (metric.value.is_some(), metric.source) {
            (true, MetricSource::Unavailable) => {
                return Err(StorageError::InvalidSample(format!(
                    "{name} has a value but unavailable provenance"
                )));
            }
            (false, source) if source != MetricSource::Unavailable => {
                return Err(StorageError::InvalidSample(format!(
                    "{name} is missing but has {source:?} provenance"
                )));
            }
            _ => {}
        }
    }

    if sample
        .metrics
        .percentage
        .value
        .is_some_and(|value| !(0.0..=100.0).contains(&value))
    {
        return Err(StorageError::InvalidSample(
            "percentage must be between 0 and 100".to_owned(),
        ));
    }
    for (name, metric) in [
        ("energy_now_wh", sample.metrics.energy_now_wh),
        ("energy_full_wh", sample.metrics.energy_full_wh),
        ("energy_design_wh", sample.metrics.energy_design_wh),
        ("voltage_volts", sample.metrics.voltage_volts),
        (
            "time_remaining_minutes",
            sample.metrics.time_remaining_minutes,
        ),
        ("cycle_count", sample.metrics.cycle_count),
    ] {
        if metric.value.is_some_and(|value| value < 0.0) {
            return Err(StorageError::InvalidSample(format!(
                "{name} cannot be negative"
            )));
        }
    }
    Ok(())
}

fn sample_metrics(sample: &NewBatterySample) -> [(&'static str, SampleMetric); 10] {
    [
        ("percentage", sample.metrics.percentage),
        ("energy_now_wh", sample.metrics.energy_now_wh),
        ("energy_full_wh", sample.metrics.energy_full_wh),
        ("energy_design_wh", sample.metrics.energy_design_wh),
        ("power_watts", sample.metrics.power_watts),
        ("voltage_volts", sample.metrics.voltage_volts),
        ("current_amps", sample.metrics.current_amps),
        ("temperature_celsius", sample.metrics.temperature_celsius),
        (
            "time_remaining_minutes",
            sample.metrics.time_remaining_minutes,
        ),
        ("cycle_count", sample.metrics.cycle_count),
    ]
}

fn is_unique_constraint(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == ErrorCode::ConstraintViolation
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        HistoryFreshness, HistoryGapReason, HistoryQuery, HistoryTimelineItem, InsertOutcome,
        MetricSource, NewBatterySample, SampleMetric, SampleMetrics, SampleState, Storage,
        StorageError, database_path_from_data_home,
    };
    use time::{OffsetDateTime, macros::datetime};

    fn temporary_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock is after UNIX epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "battery-dashboard-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn sample() -> NewBatterySample {
        let available = SampleMetric {
            value: Some(72.5),
            source: MetricSource::Upower,
        };
        let missing = SampleMetric::unavailable();
        NewBatterySample {
            battery_id: "BAT0".to_owned(),
            recorded_at: datetime!(2026-08-23 12:00 UTC),
            boot_id: "11111111-2222-3333-4444-555555555555".to_owned(),
            boot_seconds: 123.5,
            state: SampleState::Discharging,
            metrics: SampleMetrics {
                percentage: available,
                energy_now_wh: available,
                energy_full_wh: available,
                energy_design_wh: missing,
                power_watts: SampleMetric {
                    value: Some(-8.4),
                    source: MetricSource::Sysfs,
                },
                voltage_volts: available,
                current_amps: missing,
                temperature_celsius: missing,
                time_remaining_minutes: missing,
                cycle_count: missing,
            },
        }
    }

    fn sample_at(minutes: i64, percentage: Option<f64>) -> NewBatterySample {
        let mut observation = sample();
        observation.recorded_at += time::Duration::minutes(minutes);
        let seconds = minutes
            .checked_mul(60)
            .and_then(|value| i32::try_from(value).ok())
            .map(f64::from)
            .expect("test minute offset fits in f64");
        observation.boot_seconds = (observation.boot_seconds + seconds).max(0.0);
        observation.metrics.percentage =
            percentage.map_or_else(SampleMetric::unavailable, |value| SampleMetric {
                value: Some(value),
                source: MetricSource::Upower,
            });
        observation
    }

    fn history_query(battery_id: Option<&str>, max_points: usize) -> HistoryQuery {
        HistoryQuery {
            start: datetime!(2026-08-23 00:00 UTC),
            end: datetime!(2026-08-24 00:00 UTC),
            battery_id: battery_id.map(str::to_owned),
            max_points,
        }
    }

    #[test]
    fn migrates_an_empty_database_and_configures_integrity() {
        let root = temporary_path("migration");
        let path = database_path_from_data_home(&root);
        let storage = Storage::open_at(&path).expect("empty database migrates");

        assert_eq!(storage.schema_version().expect("version is readable"), 1);
        assert_eq!(storage.sample_count().expect("count is readable"), 0);
        assert_eq!(
            storage
                .last_recorded_at()
                .expect("empty history is readable"),
            None
        );
        storage.integrity_check().expect("fresh database is sound");
        let foreign_keys: i64 = storage
            .connection()
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("foreign key pragma is readable");
        assert_eq!(foreign_keys, 1);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn migration_is_idempotent_for_an_existing_database() {
        let root = temporary_path("upgrade");
        let path = database_path_from_data_home(&root);
        let first = Storage::open_at(&path).expect("first open migrates");
        drop(first);
        let second = Storage::open_at(&path).expect("second open leaves schema intact");
        assert_eq!(second.schema_version().expect("version is readable"), 1);
        drop(second);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn duplicate_samples_are_idempotent_and_rows_cannot_be_mutated() {
        let root = temporary_path("duplicates");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        let first_sample = sample();
        assert_eq!(
            storage
                .insert_sample(&first_sample)
                .expect("insert succeeds"),
            InsertOutcome::Inserted
        );
        assert_eq!(
            storage
                .last_recorded_at()
                .expect("latest sample time is readable"),
            Some(first_sample.recorded_at)
        );
        assert_eq!(
            storage
                .insert_sample(&first_sample)
                .expect("duplicate is accepted"),
            InsertOutcome::Duplicate
        );
        assert_eq!(storage.sample_count().expect("count is readable"), 1);

        let update = storage
            .connection()
            .execute("UPDATE battery_samples SET state = 'charging'", []);
        assert!(update.is_err(), "immutable trigger rejects mutation");
        storage
            .integrity_check()
            .expect("duplicate handling preserves integrity");
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn invalid_writes_are_rejected_before_any_row_is_created() {
        let root = temporary_path("invalid");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        let mut invalid = sample();
        invalid.metrics.percentage.value = Some(101.0);

        assert!(matches!(
            storage.insert_sample(&invalid),
            Err(StorageError::InvalidSample(_))
        ));
        assert_eq!(storage.sample_count().expect("count is readable"), 0);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn different_batteries_can_share_a_collection_instant() {
        let root = temporary_path("multiple-batteries");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        let first = sample();
        let mut second = sample();
        second.battery_id = "BAT1".to_owned();
        second.recorded_at =
            OffsetDateTime::from_unix_timestamp(1_777_168_000).expect("timestamp fits");

        assert_eq!(
            storage.insert_sample(&first).expect("first inserts"),
            InsertOutcome::Inserted
        );
        assert_eq!(
            storage.insert_sample(&second).expect("second inserts"),
            InsertOutcome::Inserted
        );
        assert_eq!(storage.sample_count().expect("count is readable"), 2);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn reader_and_writer_can_use_the_wal_database_concurrently() {
        let root = temporary_path("reader-writer");
        let path = database_path_from_data_home(&root);
        let mut writer = Storage::open_at(&path).expect("writer opens database");
        let reader = Storage::open_at(&path).expect("reader opens database");

        assert_eq!(
            writer
                .insert_sample(&sample())
                .expect("writer inserts sample"),
            InsertOutcome::Inserted
        );
        assert_eq!(reader.sample_count().expect("reader sees committed row"), 1);
        reader
            .integrity_check()
            .expect("concurrent access preserves integrity");
        drop(reader);
        drop(writer);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn history_respects_time_range_and_battery_filter() {
        let root = temporary_path("history-filter");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        let before = sample_at(-800, Some(99.0));
        let in_range = sample_at(1, Some(71.0));
        let mut other_battery = sample_at(2, Some(64.0));
        other_battery.battery_id = "BAT1".to_owned();
        assert_eq!(
            storage.insert_sample(&before).expect("before inserts"),
            InsertOutcome::Inserted
        );
        assert_eq!(
            storage.insert_sample(&in_range).expect("row inserts"),
            InsertOutcome::Inserted
        );
        assert_eq!(
            storage
                .insert_sample(&other_battery)
                .expect("other inserts"),
            InsertOutcome::Inserted
        );

        let result = storage
            .history(&history_query(Some("BAT0"), 50))
            .expect("history reads");
        assert_eq!(result.summary.sample_count, 1);
        assert_eq!(result.timeline.len(), 1);
        assert_eq!(result.summary.percentage.average, Some(71.0));
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn history_downsampling_preserves_endpoints_deterministically() {
        let root = temporary_path("history-downsample");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        for minute in 0_i32..10 {
            assert_eq!(
                storage
                    .insert_sample(&sample_at(
                        i64::from(minute),
                        Some(80.0 - f64::from(minute))
                    ))
                    .expect("row inserts"),
                InsertOutcome::Inserted
            );
        }
        let first = storage
            .history(&history_query(Some("BAT0"), 3))
            .expect("history reads");
        let second = storage
            .history(&history_query(Some("BAT0"), 3))
            .expect("history rereads");
        assert_eq!(first.timeline, second.timeline);
        assert_eq!(first.timeline.len(), 3);
        let timestamps = first
            .timeline
            .iter()
            .filter_map(|item| match item {
                HistoryTimelineItem::Sample(sample) => Some(sample.recorded_at.as_str()),
                HistoryTimelineItem::Gap(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(timestamps.first(), Some(&"2026-08-23T12:00:00Z"));
        assert_eq!(timestamps.last(), Some(&"2026-08-23T12:09:00Z"));
        assert_eq!(first.summary.sample_count, 10);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn history_keeps_unavailable_metrics_and_mixed_states() {
        let root = temporary_path("history-missing");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        let first = sample_at(0, None);
        let mut second = sample_at(1, Some(70.0));
        second.state = SampleState::Charging;
        second.metrics.power_watts = SampleMetric::unavailable();
        storage.insert_sample(&first).expect("first inserts");
        storage.insert_sample(&second).expect("second inserts");

        let result = storage
            .history(&history_query(Some("BAT0"), 10))
            .expect("history reads");
        let HistoryTimelineItem::Sample(first) = &result.timeline[0] else {
            panic!("first item is sample")
        };
        assert_eq!(first.metrics.percentage.value, None);
        assert_eq!(first.metrics.percentage.source, MetricSource::Unavailable);
        assert_eq!(
            first.metrics.percentage.freshness,
            HistoryFreshness::Unavailable
        );
        let HistoryTimelineItem::Sample(second) = &result.timeline[1] else {
            panic!("second item is sample")
        };
        assert_eq!(second.state, SampleState::Charging);
        assert_eq!(result.summary.percentage.minimum, Some(70.0));
        assert_eq!(result.summary.observed_energy_wh, None);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn history_marks_suspend_and_reboot_as_gaps_without_energy_estimates() {
        let root = temporary_path("history-gaps");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        let first = sample_at(0, Some(80.0));
        let suspend_gap = sample_at(10, Some(79.0));
        let mut reboot_gap = sample_at(11, Some(78.0));
        reboot_gap.boot_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_owned();
        reboot_gap.boot_seconds = 5.0;
        storage.insert_sample(&first).expect("first inserts");
        storage
            .insert_sample(&suspend_gap)
            .expect("suspend row inserts");
        storage
            .insert_sample(&reboot_gap)
            .expect("reboot row inserts");

        let result = storage
            .history(&history_query(Some("BAT0"), 1))
            .expect("history reads");
        assert_eq!(
            result.timeline.len(),
            5,
            "gap anchors override a too-small chart preference"
        );
        let reasons = result
            .timeline
            .iter()
            .filter_map(|item| match item {
                HistoryTimelineItem::Gap(gap) => Some(gap.reason),
                HistoryTimelineItem::Sample(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            reasons,
            vec![
                HistoryGapReason::SampleIntervalExceeded,
                HistoryGapReason::BootChanged
            ]
        );
        assert_eq!(result.summary.observed_duration_seconds, None);
        assert_eq!(result.summary.observed_energy_wh, None);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn empty_history_and_absent_default_database_do_not_create_data() {
        let root = temporary_path("history-empty");
        let path = database_path_from_data_home(&root);
        let storage = Storage::open_at(&path).expect("database opens");
        let result = storage
            .history(&history_query(Some("BAT0"), 10))
            .expect("empty history reads");
        assert!(result.timeline.is_empty());
        assert_eq!(result.summary.sample_count, 0);
        assert_eq!(result.summary.observed_energy_wh, None);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }
}
