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

const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/0001_initial.sql"),
    include_str!("../../migrations/0002_battery_sessions.sql"),
];

/// An error raised while locating, migrating, validating, or writing the local database.
#[derive(Debug)]
pub enum StorageError {
    /// The XDG data directory cannot be determined without guessing a path.
    DataDirectoryUnavailable,
    /// A caller supplied an invalid sample.
    InvalidSample(String),
    /// A caller requested a history interval that cannot be interpreted safely.
    InvalidHistoryQuery(String),
    /// A caller supplied an invalid derived-session or aggregation query.
    InvalidSessionQuery(String),
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
            Self::InvalidSessionQuery(message) => write!(formatter, "invalid session query: {message}"),
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
            | Self::InvalidSessionQuery(_)
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
///
/// Numeric field summaries are over the raw observations. An unfiltered query
/// must not be read as a synthetic battery: callers that need a physical
/// aggregate should combine compatible fields explicitly and preserve missing
/// values.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct HistorySummary {
    /// Number of raw, durable samples in the requested range.
    pub sample_count: usize,
    /// Wall-clock coverage across the union of contiguous observed intervals.
    /// For an unfiltered multi-battery query, overlapping battery intervals are
    /// counted once rather than multiplying the same elapsed time by battery
    /// count.
    pub observed_duration_seconds: Option<f64>,
    /// Signed watt-hours integrated per battery from power only when every
    /// observed interval has valid endpoint power and every selected battery
    /// contributes an interval. Otherwise absent; no missing battery is
    /// treated as zero.
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

/// A normalized activity represented by a derived battery session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(missing_docs)]
pub enum BatterySessionKind {
    Charging,
    Discharging,
    Full,
    /// Includes provider `idle` and `unknown`: neither is safely charge or discharge.
    Unknown,
}

impl BatterySessionKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Charging => "charging",
            Self::Discharging => "discharging",
            Self::Full => "full",
            Self::Unknown => "unknown",
        }
    }
}

/// Why a session ended without a continuously observed terminal transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionInterruptReason {
    /// The following observation reported another state. The exact transition instant is unknown.
    StateChanged,
    /// A new Linux boot makes boot-relative timing discontinuous.
    BootChanged,
    /// The recorder was absent for longer than its continuity limit.
    SampleGap,
    /// No subsequent sample exists. This can include battery removal, shutdown, or a stopped recorder.
    DataEnded,
}

impl SessionInterruptReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::StateChanged => "state_changed",
            Self::BootChanged => "boot_changed",
            Self::SampleGap => "sample_gap",
            Self::DataEnded => "data_ended",
        }
    }
}

/// A durable, derived contiguous run of observations for one physical battery.
///
/// `observed_duration_seconds` is the sum of only adjacent observed intervals.
/// It is never extended to the next state, a gap, or the current time.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct BatterySession {
    pub battery_id: String,
    pub kind: BatterySessionKind,
    pub started_at: String,
    pub ended_at: String,
    pub sample_count: u64,
    pub observed_duration_seconds: Option<f64>,
    /// Endpoint values are present only when every sample in this session had
    /// that metric; they never bridge a missing provider value.
    pub start_percentage: Option<f64>,
    pub end_percentage: Option<f64>,
    pub start_energy_wh: Option<f64>,
    pub end_energy_wh: Option<f64>,
    /// Time-weighted observed power, only when every sample supplied power.
    pub average_power_watts: Option<f64>,
    /// True only when a following observed sample establishes a state boundary.
    pub complete: bool,
    pub interrupt_reason: SessionInterruptReason,
}

/// A bounded session read. Supplying no battery selects sessions per battery,
/// never a synthetic combined battery.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct SessionQuery {
    pub start: OffsetDateTime,
    pub end: OffsetDateTime,
    pub battery_id: Option<String>,
}

/// Calendar bucket width for observed session-duration reporting.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(missing_docs)]
pub enum SessionAggregationPeriod {
    Daily,
    Weekly,
    Monthly,
}

/// A per-battery calendar bucket. Metrics are intentionally absent: sessions
/// cannot establish energy or power across unobserved boundaries.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct SessionAggregation {
    /// ISO-8601 local calendar key (`YYYY-MM-DD`, `YYYY-Www`, or `YYYY-MM`).
    pub bucket: String,
    pub battery_id: String,
    pub session_count: u64,
    pub complete_session_count: u64,
    pub observed_duration_seconds: Option<f64>,
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

    /// Rebuilds all derived sessions from immutable `battery_samples`.
    ///
    /// This is deliberately a whole-table, transactional rebuild rather than a
    /// best-effort incremental cache: running it repeatedly produces the same
    /// rows from the same samples and cannot alter the source observations.
    ///
    /// # Errors
    ///
    /// Returns an error when source rows cannot be read or the replacement
    /// derived rows cannot be committed.
    pub fn rebuild_sessions(&mut self) -> Result<u64, StorageError> {
        let samples = load_session_samples(&self.connection)?;
        let sessions = derive_sessions(&samples);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("DELETE FROM battery_sessions", [])?;
        for session in &sessions {
            transaction.execute(
                "INSERT INTO battery_sessions (
                    battery_id, kind, started_at_utc, ended_at_utc, sample_count,
                    observed_duration_seconds, start_percentage, end_percentage,
                    start_energy_wh, end_energy_wh, average_power_watts, complete, interrupt_reason
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    session.battery_id,
                    session.kind.as_str(),
                    session.started_at,
                    session.ended_at,
                    session.sample_count,
                    session.observed_duration_seconds,
                    session.start_percentage,
                    session.end_percentage,
                    session.start_energy_wh,
                    session.end_energy_wh,
                    session.average_power_watts,
                    session.complete,
                    session.interrupt_reason.as_str(),
                ],
            )?;
        }
        transaction.commit()?;
        u64::try_from(sessions.len())
            .map_err(|_| StorageError::InvalidSessionQuery("session count exceeds u64".to_owned()))
    }

    /// Reads derived sessions whose observed range overlaps the inclusive UTC query.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid bounds or unreadable derived rows.
    pub fn sessions(&self, query: &SessionQuery) -> Result<Vec<BatterySession>, StorageError> {
        validate_session_query(query)?;
        let start = format_utc(query.start)?;
        let end = format_utc(query.end)?;
        let mut statement = self.connection.prepare(
            "SELECT battery_id, kind, started_at_utc, ended_at_utc, sample_count,
                    observed_duration_seconds, start_percentage, end_percentage,
                    start_energy_wh, end_energy_wh, average_power_watts, complete, interrupt_reason
             FROM battery_sessions
             WHERE ended_at_utc >= ?1 AND started_at_utc <= ?2
               AND (?3 IS NULL OR battery_id = ?3)
             ORDER BY started_at_utc ASC, id ASC",
        )?;
        statement
            .query_map(
                params![start, end, query.battery_id.as_deref()],
                session_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Groups sessions by the local calendar date of their first observation.
    ///
    /// `offset` is a fixed offset, so callers requiring IANA timezone/DST rules
    /// must perform grouping in a timezone-aware frontend or add a dedicated
    /// timezone dependency. A bucket whose session crosses a calendar boundary
    /// reports no duration rather than allocating unobserved partial time.
    ///
    /// # Errors
    ///
    /// Returns an error when the session query is invalid or derived data is malformed.
    pub fn aggregate_sessions(
        &self,
        query: &SessionQuery,
        period: SessionAggregationPeriod,
        offset: time::UtcOffset,
    ) -> Result<Vec<SessionAggregation>, StorageError> {
        use std::collections::BTreeMap;

        let sessions = self.sessions(query)?;
        let mut buckets = BTreeMap::<(String, String), (u64, u64, Option<f64>)>::new();
        for session in sessions {
            let start = OffsetDateTime::parse(&session.started_at, &Rfc3339)
                .map_err(|error| StorageError::InvalidSessionQuery(error.to_string()))?
                .to_offset(offset);
            let end = OffsetDateTime::parse(&session.ended_at, &Rfc3339)
                .map_err(|error| StorageError::InvalidSessionQuery(error.to_string()))?
                .to_offset(offset);
            let bucket = session_bucket(start, period);
            let crosses_boundary = bucket != session_bucket(end, period);
            let entry = buckets
                .entry((bucket, session.battery_id))
                .or_insert((0, 0, Some(0.0)));
            entry.0 += 1;
            entry.1 += u64::from(session.complete);
            entry.2 = match (entry.2, session.observed_duration_seconds, crosses_boundary) {
                (Some(total), Some(duration), false) => Some(total + duration),
                _ => None,
            };
        }
        Ok(buckets
            .into_iter()
            .map(
                |(
                    (bucket, battery_id),
                    (session_count, complete_session_count, observed_duration_seconds),
                )| SessionAggregation {
                    bucket,
                    battery_id,
                    session_count,
                    complete_session_count,
                    observed_duration_seconds,
                },
            )
            .collect())
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
    configure_read_connection(&connection)?;
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

/// Returns the duration that can safely be attributed to two adjacent
/// observations from one boot. Wall-clock time is used to reject clock
/// reversals and long absences, while boot-relative time supplies the duration
/// itself so small wall-clock adjustments do not create or erase observed time.
fn contiguous_interval_seconds(
    first_recorded_at: OffsetDateTime,
    second_recorded_at: OffsetDateTime,
    first_boot_id: &str,
    second_boot_id: &str,
    first_boot_seconds: f64,
    second_boot_seconds: f64,
) -> Option<f64> {
    if first_boot_id != second_boot_id {
        return None;
    }
    let wall_seconds = (second_recorded_at - first_recorded_at).as_seconds_f64();
    let boot_seconds = second_boot_seconds - first_boot_seconds;
    if !wall_seconds.is_finite()
        || !boot_seconds.is_finite()
        || wall_seconds <= 0.0
        || boot_seconds <= 0.0
        || wall_seconds > MAX_CONTIGUOUS_SAMPLE_SECONDS
        || boot_seconds > MAX_CONTIGUOUS_SAMPLE_SECONDS
    {
        return None;
    }
    Some(boot_seconds)
}

fn history_interval_seconds(first: &RawHistorySample, second: &RawHistorySample) -> Option<f64> {
    contiguous_interval_seconds(
        first.recorded_at,
        second.recorded_at,
        &first.sample.boot_id,
        &second.sample.boot_id,
        first.sample.boot_seconds,
        second.sample.boot_seconds,
    )
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

#[derive(Clone, Debug)]
struct RawSessionSample {
    battery_id: String,
    recorded_at: OffsetDateTime,
    state: SampleState,
    boot_id: String,
    boot_seconds: f64,
    percentage: Option<f64>,
    energy_now_wh: Option<f64>,
    power_watts: Option<f64>,
}

fn load_session_samples(connection: &Connection) -> Result<Vec<RawSessionSample>, StorageError> {
    let mut statement = connection.prepare(
        "SELECT battery_id, recorded_at_utc, state, boot_id, boot_seconds,
                percentage, energy_now_wh, power_watts
         FROM battery_samples ORDER BY battery_id ASC, recorded_at_utc ASC, id ASC",
    )?;
    statement
        .query_map([], |row| {
            let timestamp: String = row.get(1)?;
            let recorded_at = OffsetDateTime::parse(&timestamp, &Rfc3339).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(RawSessionSample {
                battery_id: row.get(0)?,
                recorded_at: recorded_at.to_offset(time::UtcOffset::UTC),
                state: sample_state_from_database(&row.get::<_, String>(2)?)?,
                boot_id: row.get(3)?,
                boot_seconds: row.get(4)?,
                percentage: row.get(5)?,
                energy_now_wh: row.get(6)?,
                power_watts: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn session_kind(state: SampleState) -> BatterySessionKind {
    match state {
        SampleState::Charging => BatterySessionKind::Charging,
        SampleState::Discharging => BatterySessionKind::Discharging,
        SampleState::Full => BatterySessionKind::Full,
        SampleState::Idle | SampleState::Unknown => BatterySessionKind::Unknown,
    }
}

fn derive_sessions(samples: &[RawSessionSample]) -> Vec<BatterySession> {
    let mut output = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        let battery_id = &samples[start].battery_id;
        let mut end = start + 1;
        while end < samples.len() && samples[end].battery_id == *battery_id {
            end += 1;
        }
        let battery_samples = &samples[start..end];
        let mut segment_start = 0;
        for index in 1..battery_samples.len() {
            let previous = &battery_samples[index - 1];
            let current = &battery_samples[index];
            let reason = if previous.boot_id != current.boot_id {
                Some(SessionInterruptReason::BootChanged)
            } else if contiguous_interval_seconds(
                previous.recorded_at,
                current.recorded_at,
                &previous.boot_id,
                &current.boot_id,
                previous.boot_seconds,
                current.boot_seconds,
            )
            .is_none()
            {
                Some(SessionInterruptReason::SampleGap)
            } else if previous.state != current.state {
                Some(SessionInterruptReason::StateChanged)
            } else {
                None
            };
            if let Some(reason) = reason {
                output.push(build_session(
                    &battery_samples[segment_start..index],
                    reason,
                ));
                segment_start = index;
            }
        }
        output.push(build_session(
            &battery_samples[segment_start..],
            SessionInterruptReason::DataEnded,
        ));
        start = end;
    }
    output
}

fn build_session(
    samples: &[RawSessionSample],
    interrupt_reason: SessionInterruptReason,
) -> BatterySession {
    debug_assert!(!samples.is_empty());
    let first = &samples[0];
    let last = samples.last().expect("nonempty session");
    let intervals = samples
        .windows(2)
        .map(|pair| {
            contiguous_interval_seconds(
                pair[0].recorded_at,
                pair[1].recorded_at,
                &pair[0].boot_id,
                &pair[1].boot_id,
                pair[0].boot_seconds,
                pair[1].boot_seconds,
            )
        })
        .collect::<Option<Vec<_>>>();
    let duration = intervals.as_ref().and_then(|intervals| {
        let total = intervals.iter().sum::<f64>();
        (total > 0.0).then_some(total)
    });
    let complete_metric = |values: Vec<Option<f64>>| values.into_iter().collect::<Option<Vec<_>>>();
    let percentages = complete_metric(samples.iter().map(|sample| sample.percentage).collect());
    let energy = complete_metric(samples.iter().map(|sample| sample.energy_now_wh).collect());
    let powers = complete_metric(samples.iter().map(|sample| sample.power_watts).collect());
    let average_power_watts = match (duration, intervals, powers) {
        (Some(seconds), Some(intervals), Some(powers)) if seconds > 0.0 => Some(
            intervals
                .iter()
                .zip(powers.windows(2))
                .map(|(interval, power)| power[0].midpoint(power[1]) * interval)
                .sum::<f64>()
                / seconds,
        ),
        _ => None,
    };
    BatterySession {
        battery_id: first.battery_id.clone(),
        kind: session_kind(first.state),
        started_at: format_utc(first.recorded_at).expect("database timestamp was parsed"),
        ended_at: format_utc(last.recorded_at).expect("database timestamp was parsed"),
        sample_count: u64::try_from(samples.len()).expect("slice length fits u64"),
        observed_duration_seconds: duration,
        start_percentage: percentages.as_ref().map(|values| values[0]),
        end_percentage: percentages
            .as_ref()
            .and_then(|values| values.last().copied()),
        start_energy_wh: energy.as_ref().map(|values| values[0]),
        end_energy_wh: energy.as_ref().and_then(|values| values.last().copied()),
        average_power_watts,
        complete: interrupt_reason == SessionInterruptReason::StateChanged,
        interrupt_reason,
    }
}

fn session_from_row(row: &Row<'_>) -> rusqlite::Result<BatterySession> {
    Ok(BatterySession {
        battery_id: row.get(0)?,
        kind: match row.get::<_, String>(1)?.as_str() {
            "charging" => BatterySessionKind::Charging,
            "discharging" => BatterySessionKind::Discharging,
            "full" => BatterySessionKind::Full,
            "unknown" => BatterySessionKind::Unknown,
            _value => {
                return Err(rusqlite::Error::InvalidColumnType(
                    1,
                    "kind".to_owned(),
                    rusqlite::types::Type::Text,
                ));
            }
        },
        started_at: row.get(2)?,
        ended_at: row.get(3)?,
        sample_count: row.get(4)?,
        observed_duration_seconds: row.get(5)?,
        start_percentage: row.get(6)?,
        end_percentage: row.get(7)?,
        start_energy_wh: row.get(8)?,
        end_energy_wh: row.get(9)?,
        average_power_watts: row.get(10)?,
        complete: row.get(11)?,
        interrupt_reason: match row.get::<_, String>(12)?.as_str() {
            "state_changed" => SessionInterruptReason::StateChanged,
            "boot_changed" => SessionInterruptReason::BootChanged,
            "sample_gap" => SessionInterruptReason::SampleGap,
            "data_ended" => SessionInterruptReason::DataEnded,
            _ => {
                return Err(rusqlite::Error::InvalidColumnType(
                    12,
                    "interrupt_reason".to_owned(),
                    rusqlite::types::Type::Text,
                ));
            }
        },
    })
}

fn validate_session_query(query: &SessionQuery) -> Result<(), StorageError> {
    if query.start > query.end {
        return Err(StorageError::InvalidSessionQuery(
            "start must not be after end".to_owned(),
        ));
    }
    if query
        .battery_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(StorageError::InvalidSessionQuery(
            "battery_id must not be empty when supplied".to_owned(),
        ));
    }
    Ok(())
}

fn session_bucket(timestamp: OffsetDateTime, period: SessionAggregationPeriod) -> String {
    let date = timestamp.date();
    match period {
        SessionAggregationPeriod::Daily => date.to_string(),
        SessionAggregationPeriod::Monthly => {
            format!("{:04}-{:02}", date.year(), u8::from(date.month()))
        }
        SessionAggregationPeriod::Weekly => {
            let (year, week, _) = date.to_iso_week_date();
            format!("{year:04}-W{week:02}")
        }
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
    let selected = if query.battery_id.is_none() {
        downsample_timestamp_groups(raw, query.max_points, &gaps)
    } else {
        downsample_indices(raw.len(), query.max_points, &gaps)
    };
    let mut timeline = Vec::with_capacity(selected.len() + gaps.len());
    for index in selected {
        timeline.push(HistoryTimelineItem::Sample(Box::new(
            raw[index].sample.clone(),
        )));
        let gap_anchor = if query.battery_id.is_none() {
            (timestamp_group_end(raw, index) == index + 1).then_some(index)
        } else {
            Some(index)
        };
        if let Some(gap_anchor) = gap_anchor {
            for (_, gap) in gaps.iter().filter(|(gap_index, _)| {
                if query.battery_id.is_none() {
                    // An aggregate timestamp group may contain several
                    // battery rows.  A gap belongs after its *entire*
                    // source instant, rather than after the one row for
                    // the battery that exposed it.
                    timestamp_group_end(raw, *gap_index) - 1 == gap_anchor
                } else {
                    *gap_index == gap_anchor
                }
            }) {
                timeline.push(HistoryTimelineItem::Gap(gap.clone()));
            }
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
                    } else if history_interval_seconds(first, second).is_none() {
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

/// Downsamples an aggregate history by collection instant rather than by raw
/// row. Every physical battery observed at a selected instant stays together;
/// otherwise a compact aggregate chart can accidentally combine half of one
/// instant with half of another and report a false missing-battery gap.
fn downsample_timestamp_groups(
    raw: &[RawHistorySample],
    max_points: usize,
    gaps: &[(usize, HistoryGap)],
) -> Vec<usize> {
    if raw.is_empty() {
        return Vec::new();
    }

    let mut groups = Vec::<(usize, usize)>::new();
    let mut start = 0;
    while start < raw.len() {
        let end = timestamp_group_end(raw, start);
        groups.push((start, end));
        start = end;
    }

    let group_for_row = |row_index: usize| {
        groups
            .iter()
            .position(|(group_start, group_end)| {
                *group_start <= row_index && row_index < *group_end
            })
            .expect("every raw history row belongs to a timestamp group")
    };
    let group_gaps = gaps
        .iter()
        .map(|(row_index, _)| {
            (
                group_for_row(*row_index),
                HistoryGap {
                    from: String::new(),
                    to: String::new(),
                    reason: HistoryGapReason::SampleIntervalExceeded,
                },
            )
        })
        .collect::<Vec<_>>();
    let selected_groups = downsample_indices(groups.len(), max_points, &group_gaps);
    selected_groups
        .into_iter()
        .flat_map(|group_index| groups[group_index].0..groups[group_index].1)
        .collect()
}

fn timestamp_group_end(raw: &[RawHistorySample], start: usize) -> usize {
    let timestamp = &raw[start].sample.recorded_at;
    let mut end = start + 1;
    while end < raw.len() && raw[end].sample.recorded_at == *timestamp {
        end += 1;
    }
    end
}

fn history_summary(raw: &[RawHistorySample], gaps: &[(usize, HistoryGap)]) -> HistorySummary {
    use std::collections::{BTreeMap, BTreeSet};

    let discontinuities = gaps
        .iter()
        .map(|(index, _)| *index)
        .collect::<BTreeSet<_>>();
    let mut intervals = Vec::<(OffsetDateTime, OffsetDateTime, f64)>::new();
    let mut observed_energy_watt_seconds = 0.0;
    let mut energy_supported = raw.len() >= 2;
    let mut battery_ids = BTreeSet::<&str>::new();
    let mut battery_has_interval = BTreeMap::<&str, bool>::new();
    for sample in raw {
        battery_ids.insert(sample.battery_id.as_str());
        battery_has_interval
            .entry(sample.battery_id.as_str())
            .or_insert(false);
    }
    let mut indices_by_battery = BTreeMap::<&str, Vec<usize>>::new();
    for (index, sample) in raw.iter().enumerate() {
        indices_by_battery
            .entry(sample.battery_id.as_str())
            .or_default()
            .push(index);
    }
    for indices in indices_by_battery.values() {
        for pair in indices.windows(2) {
            let first_index = pair[0];
            let second_index = pair[1];
            if discontinuities.contains(&first_index) {
                continue;
            }
            let first = &raw[first_index];
            let second = &raw[second_index];
            let Some(seconds) = history_interval_seconds(first, second) else {
                energy_supported = false;
                continue;
            };
            intervals.push((first.recorded_at, second.recorded_at, seconds));
            battery_has_interval.insert(first.battery_id.as_str(), true);
            match (
                first.sample.metrics.power_watts.value,
                second.sample.metrics.power_watts.value,
            ) {
                (Some(first_power), Some(second_power)) => {
                    observed_energy_watt_seconds += first_power.midpoint(second_power) * seconds;
                }
                _ => energy_supported = false,
            }
        }
    }
    let observed_seconds = union_observed_duration(&intervals);
    let all_batteries_have_intervals = battery_ids.iter().all(|battery_id| {
        battery_has_interval
            .get(battery_id)
            .copied()
            .unwrap_or(false)
    });
    HistorySummary {
        sample_count: raw.len(),
        observed_duration_seconds: observed_seconds,
        observed_energy_wh: (energy_supported
            && all_batteries_have_intervals
            && !intervals.is_empty())
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

fn union_observed_duration(intervals: &[(OffsetDateTime, OffsetDateTime, f64)]) -> Option<f64> {
    if intervals.is_empty() {
        return None;
    }
    let mut ordered = intervals.to_vec();
    ordered.sort_by_key(|(start, end, _)| (*start, *end));

    let mut total = 0.0;
    let mut current_start = ordered[0].0;
    let mut current_end = ordered[0].1;
    for (start, end, _) in ordered.into_iter().skip(1) {
        if start <= current_end {
            if end > current_end {
                current_end = end;
            }
        } else {
            total += (current_end - current_start).as_seconds_f64();
            current_start = start;
            current_end = end;
        }
    }
    total += (current_end - current_start).as_seconds_f64();
    (total > 0.0).then_some(total)
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
                && matches!(
                    failure.extended_code,
                    rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
                        | rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                )
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
        BatterySessionKind, HistoryFreshness, HistoryGapReason, HistoryQuery, HistoryTimelineItem,
        InsertOutcome, MAX_CONTIGUOUS_SAMPLE_SECONDS, MetricSource, NewBatterySample, SampleMetric,
        SampleMetrics, SampleState, SessionAggregationPeriod, SessionInterruptReason, SessionQuery,
        Storage, StorageError, database_path_from_data_home, session_bucket,
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

        assert_eq!(storage.schema_version().expect("version is readable"), 2);
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
        assert_eq!(second.schema_version().expect("version is readable"), 2);
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
    fn aggregate_history_keeps_batteries_together_and_does_not_cross_join_intervals() {
        let root = temporary_path("history-multiple-aggregate");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        let first = sample_at(0, Some(80.0));
        let second = sample_at(1, Some(79.0));
        let mut other_first = first.clone();
        other_first.battery_id = "BAT1".to_owned();
        let mut other_second = second.clone();
        other_second.battery_id = "BAT1".to_owned();
        for sample in [first, other_first, second, other_second] {
            storage.insert_sample(&sample).expect("sample inserts");
        }

        let result = storage
            .history(&history_query(None, 1))
            .expect("aggregate history reads");
        let samples = result
            .timeline
            .iter()
            .filter(|item| matches!(item, HistoryTimelineItem::Sample(_)))
            .count();
        assert_eq!(samples, 4, "one selected instant retains every battery row");
        assert_eq!(result.summary.observed_duration_seconds, Some(60.0));
        assert_eq!(result.summary.observed_energy_wh, Some(-0.28));
        assert!(
            result
                .timeline
                .iter()
                .all(|item| !matches!(item, HistoryTimelineItem::Gap(_)))
        );

        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn aggregate_history_places_a_battery_gap_after_the_shared_instant() {
        let root = temporary_path("history-aggregate-gap-placement");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        let first = sample_at(0, Some(80.0));
        let mut other_first = first.clone();
        other_first.battery_id = "BAT1".to_owned();
        let mut other_second = sample_at(1, Some(79.0));
        other_second.battery_id = "BAT1".to_owned();
        let second = sample_at(10, Some(70.0));
        for sample in [first, other_first, other_second, second] {
            storage.insert_sample(&sample).expect("sample inserts");
        }

        let result = storage
            .history(&history_query(None, 10))
            .expect("aggregate history reads");
        assert!(matches!(
            result.timeline.as_slice(),
            [
                HistoryTimelineItem::Sample(_),
                HistoryTimelineItem::Sample(_),
                HistoryTimelineItem::Gap(_),
                HistoryTimelineItem::Sample(_),
                HistoryTimelineItem::Sample(_),
            ]
        ));

        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn monotonic_boot_gap_breaks_history_even_when_wall_time_is_short() {
        let root = temporary_path("history-monotonic-gap");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        let first = sample_at(0, Some(80.0));
        let mut second = sample_at(1, Some(79.0));
        second.boot_seconds += MAX_CONTIGUOUS_SAMPLE_SECONDS + 1.0;
        storage.insert_sample(&first).expect("first inserts");
        storage.insert_sample(&second).expect("second inserts");

        let result = storage
            .history(&history_query(Some("BAT0"), 10))
            .expect("history reads");
        assert_eq!(
            result
                .timeline
                .iter()
                .filter_map(|item| match item {
                    HistoryTimelineItem::Gap(gap) => Some(gap.reason),
                    HistoryTimelineItem::Sample(_) => None,
                })
                .collect::<Vec<_>>(),
            vec![HistoryGapReason::SampleIntervalExceeded]
        );
        assert_eq!(result.summary.observed_duration_seconds, None);
        assert_eq!(result.summary.observed_energy_wh, None);

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

    #[test]
    fn session_rebuild_is_idempotent_and_respects_boundaries_per_battery() {
        let root = temporary_path("sessions");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        let mut rows = vec![sample_at(0, Some(80.0)), sample_at(1, Some(79.0))];
        rows[1].metrics.energy_now_wh = SampleMetric::unavailable();
        let mut charging = sample_at(2, Some(79.5));
        charging.state = SampleState::Charging;
        rows.push(charging);
        let mut gap = sample_at(10, Some(82.0));
        gap.state = SampleState::Charging;
        rows.push(gap);
        let mut reboot = sample_at(11, Some(81.0));
        reboot.boot_id = "other-boot".to_owned();
        reboot.state = SampleState::Full;
        rows.push(reboot);
        let mut other_battery = sample_at(1, Some(50.0));
        other_battery.battery_id = "BAT1".to_owned();
        other_battery.state = SampleState::Full;
        rows.push(other_battery);
        for row in &rows {
            storage.insert_sample(row).expect("sample inserts");
        }

        assert_eq!(storage.rebuild_sessions().expect("first rebuild"), 5);
        let query = SessionQuery {
            start: datetime!(2026-08-23 00:00 UTC),
            end: datetime!(2026-08-24 00:00 UTC),
            battery_id: None,
        };
        let first = storage.sessions(&query).expect("sessions read");
        assert_eq!(storage.rebuild_sessions().expect("second rebuild"), 5);
        assert_eq!(storage.sessions(&query).expect("sessions reread"), first);
        let facts = first
            .iter()
            .map(|session| {
                (
                    session.battery_id.as_str(),
                    session.kind,
                    session.interrupt_reason,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            facts,
            vec![
                (
                    "BAT0",
                    BatterySessionKind::Discharging,
                    SessionInterruptReason::StateChanged
                ),
                (
                    "BAT1",
                    BatterySessionKind::Full,
                    SessionInterruptReason::DataEnded
                ),
                (
                    "BAT0",
                    BatterySessionKind::Charging,
                    SessionInterruptReason::SampleGap
                ),
                (
                    "BAT0",
                    BatterySessionKind::Charging,
                    SessionInterruptReason::BootChanged
                ),
                (
                    "BAT0",
                    BatterySessionKind::Full,
                    SessionInterruptReason::DataEnded
                ),
            ]
        );
        assert_eq!(first[0].observed_duration_seconds, Some(60.0));
        assert_eq!(first[0].start_energy_wh, None);
        assert_eq!(first[0].end_energy_wh, None);
        assert!(first[0].complete);
        assert!(!first[1].complete);
        assert_eq!(first[1].observed_duration_seconds, None);
        for period in [
            SessionAggregationPeriod::Daily,
            SessionAggregationPeriod::Weekly,
            SessionAggregationPeriod::Monthly,
        ] {
            let aggregates = storage
                .aggregate_sessions(&query, period, time::UtcOffset::UTC)
                .expect("aggregation reads");
            assert_eq!(aggregates.len(), 2, "never combines BAT0 and BAT1");
        }
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn aggregation_buckets_honor_the_explicit_fixed_offset() {
        let utc = datetime!(2026-08-01 00:30 UTC);
        let west = time::UtcOffset::from_hms(-1, 0, 0).expect("valid offset");
        let cases = [
            (SessionAggregationPeriod::Daily, "2026-07-31"),
            (SessionAggregationPeriod::Weekly, "2026-W31"),
            (SessionAggregationPeriod::Monthly, "2026-07"),
        ];
        for (period, expected) in cases {
            assert_eq!(session_bucket(utc.to_offset(west), period), expected);
        }
    }
}
