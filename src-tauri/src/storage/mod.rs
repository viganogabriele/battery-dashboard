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

/// Starting-charge percentage bands used to summarize observed discharge
/// durations (`DEVELOPMENT_PLAN.md` section 23.2, "duration by
/// starting-charge bands"). Boundaries are simple, round numbers a user can
/// read at a glance. Each tuple is `(low, high)`; membership is `low <=
/// start_percentage < high`, except the top band, which also includes
/// exactly 100%.
const STARTING_CHARGE_BANDS: [(f64, f64); 6] = [
    (95.0, 100.0),
    (80.0, 95.0),
    (60.0, 80.0),
    (40.0, 60.0),
    (20.0, 40.0),
    (0.0, 20.0),
];

/// Minimum starting percentage for a completed discharge session to count as
/// having begun "on a full charge" for the headline estimate.
///
/// A recorded sample is a point-in-time read, and a laptop is essentially
/// never unplugged at the exact instant a provider reports precisely 100%:
/// requiring an exact match would discard realistic evidence and could leave
/// the headline permanently empty even on a laptop that is reliably charged
/// to full before use. 95% keeps the headline restricted to sessions that
/// genuinely began at a full or near-full charge (this is also the top
/// `STARTING_CHARGE_BANDS` boundary) without an unreasonably strict
/// exact-100%-only rule.
pub const FULL_CHARGE_BAND_MIN_PERCENT: f64 = 95.0;

/// Maximum ending percentage for a completed discharge session to count as
/// having actually drained the battery, rather than being interrupted early
/// by the user plugging back in. Only sessions that reach at or below this
/// level answer "how long did the charge last"; a session that starts near
/// full but ends at, say, 70% because the laptop was plugged back in quickly
/// says nothing about full battery life and must not inflate the headline
/// average.
pub const FULLY_DRAINED_MAX_PERCENT: f64 = 20.0;

/// Observed duration statistics, in minutes, for a set of completed discharge
/// sessions. `None` at the call site (not this type) represents "no
/// qualifying sessions"; this type itself is only ever constructed from at
/// least one observed duration.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct DurationStatsMinutes {
    pub count: u64,
    pub average_minutes: f64,
    pub median_minutes: f64,
    pub min_minutes: f64,
    pub max_minutes: f64,
}

/// Observed discharge-duration evidence for one starting-charge band.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct StartingChargeBandSummary {
    pub band_start_percent: f64,
    pub band_end_percent: f64,
    /// True for the band used to answer "battery life on a full charge".
    pub is_full_charge_band: bool,
    /// Every completed discharge session that started in this band,
    /// regardless of how the session ended.
    pub all_sessions: Option<DurationStatsMinutes>,
    /// The subset of `all_sessions` that also ended at or below
    /// `FULLY_DRAINED_MAX_PERCENT`, i.e. sessions that were not interrupted
    /// early by the user plugging back in.
    pub fully_drained: Option<DurationStatsMinutes>,
}

/// Observed discharge durations grouped by starting-charge band, built only
/// from completed sessions with a known start percentage and observed
/// duration. See `STARTING_CHARGE_BANDS`, `FULL_CHARGE_BAND_MIN_PERCENT`, and
/// `FULLY_DRAINED_MAX_PERCENT` for the thresholds applied.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct DischargeDurationByStartingBand {
    /// The requested scope; `None` means sessions were pooled across every
    /// physical battery.
    pub battery_id: Option<String>,
    pub bands: Vec<StartingChargeBandSummary>,
    /// Total completed discharge sessions considered across every band.
    pub session_count: u64,
    pub earliest_session_started_at: Option<String>,
    pub latest_session_ended_at: Option<String>,
}

/// Observed percent-per-hour rate statistics for a set of completed
/// sessions. Mirrors [`DurationStatsMinutes`], but for the rate of charge
/// change rather than for elapsed duration.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct RatePercentPerHourStats {
    pub count: u64,
    pub average_percent_per_hour: f64,
    pub median_percent_per_hour: f64,
    pub min_percent_per_hour: f64,
    pub max_percent_per_hour: f64,
}

/// Observed rate evidence for the `STARTING_CHARGE_BANDS` band containing a
/// particular "current percentage right now" — the input to the live
/// runtime forecast (`main::get_runtime_forecast`).
///
/// Band selection is exact, not interpolated: the band containing the
/// current percentage is looked up with the same `low <= x < high` rule
/// (top band inclusive of 100) already used by
/// `discharge_duration_by_starting_band`, and only sessions that themselves
/// *started* in that same band are considered. A current percentage close to
/// a band edge is never blended with the neighboring band: this codebase
/// does not borrow evidence across starting-charge bands anywhere else
/// either (`discharge_duration_by_starting_band`'s headline uses only its
/// own top band), and a laptop's discharge/charge curve is not assumed to be
/// flat enough across a whole 15-20 point percentage range to make
/// cross-band interpolation safe. When the exact band has no qualifying
/// sessions yet, `stats` is `None` and the caller must say so plainly
/// ("not enough recorded history at this charge level yet") instead of
/// guessing from a neighboring band or a global average.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct HistoricalRateBand {
    pub band_start_percent: f64,
    pub band_end_percent: f64,
    pub stats: Option<RatePercentPerHourStats>,
}

/// A short, contiguous, same-boot recent trend in observed percentage read
/// from the immutable sample buffer — "how has this laptop actually been
/// used in the last little while". Blended with the historical band rate by
/// [`blend_rate_percent_per_hour`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct RecentRateEvidence {
    pub rate_percent_per_hour: f64,
    pub window_minutes: f64,
    pub sample_count: usize,
}

/// Minimum valid observations required before a day's usage summary is
/// treated as sufficient evidence, matching the local runtime-estimate
/// evidence policy in `DEVELOPMENT_PLAN.md` section 11.
const MIN_DAY_USAGE_SAMPLES: usize = 10;
/// Minimum union-observed coverage, in seconds, required before a day's usage
/// summary is treated as sufficient evidence (ten minutes).
const MIN_DAY_USAGE_COVERED_SECONDS: f64 = 600.0;

/// A bounded, timezone-resolved local-calendar-day usage query.
///
/// Callers resolve `start`/`end` from local midnight boundaries (see the
/// existing IANA/DST-aware bucketing already used for session history) and
/// pass the resulting UTC instants here. Supplying `battery_id: None` reads
/// every physical battery's raw samples together only for the coverage and
/// net-energy figures that remain valid when combined (see
/// `DayUsageSummary`); it never fabricates a combined percentage series.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct DayUsageQuery {
    pub battery_id: Option<String>,
    pub start: OffsetDateTime,
    pub end: OffsetDateTime,
}

/// Whether a day's usage summary clears the local evidence policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DayEvidence {
    /// At least ten observations spanning at least ten observed minutes.
    Sufficient,
    /// Below the evidence policy; see `DayUsageSummary::insufficiency_reason`.
    Insufficient,
}

/// Why a day did not clear the evidence policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DayInsufficiencyReason {
    /// No samples were recorded in the requested local day at all. This is the
    /// expected state for "yesterday" on a fresh install, not an error.
    NoRecording,
    /// Some samples exist, but too few observations or too little observed
    /// coverage exist to report a confident summary.
    TooFewSamples,
}

/// An observed, gap-respecting usage summary for one local calendar day.
///
/// Every derived field is `None` unless it is directly supported by the
/// underlying observations for the requested scope. Percentage and
/// discharge/charge power fields are only ever populated for a single
/// physical battery (`battery_id: Some(_)`): combining raw percentage or
/// power series across distinct physical batteries by timestamp is not
/// attempted here.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct DayUsageSummary {
    pub battery_id: Option<String>,
    pub sample_count: usize,
    /// Wall-clock span of the requested day query (bounded by "now" for the
    /// current, still-open day).
    pub elapsed_seconds: f64,
    /// Union of contiguous observed intervals; never extended across a
    /// reboot, suspend, or sampling-gap boundary.
    pub observed_duration_seconds: Option<f64>,
    /// `observed_duration_seconds / elapsed_seconds`, when both are known.
    pub coverage_ratio: Option<f64>,
    pub start_percentage: Option<f64>,
    pub end_percentage: Option<f64>,
    pub percentage_change: Option<f64>,
    /// Net watt-hours integrated from recorded power over observed,
    /// gap-respecting intervals only. Positive means net charge.
    pub energy_change_wh: Option<f64>,
    /// The largest recorded full-capacity reading seen during the day, useful
    /// for capacity-weighted multi-battery combination by a caller.
    pub representative_full_energy_wh: Option<f64>,
    pub average_discharge_power_watts: Option<f64>,
    pub average_charge_power_watts: Option<f64>,
    pub evidence: DayEvidence,
    pub insufficiency_reason: Option<DayInsufficiencyReason>,
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

    /// Returns every distinct physical battery identifier with at least one
    /// sample inside an inclusive UTC range, ordered for deterministic output.
    ///
    /// # Errors
    ///
    /// Returns an error when `SQLite` cannot read the sample table.
    pub fn battery_ids_in_range(
        &self,
        start: OffsetDateTime,
        end: OffsetDateTime,
    ) -> Result<Vec<String>, StorageError> {
        let start = format_utc(start)?;
        let end = format_utc(end)?;
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT battery_id FROM battery_samples
             WHERE recorded_at_utc >= ?1 AND recorded_at_utc <= ?2
             ORDER BY battery_id ASC",
        )?;
        statement
            .query_map(params![start, end], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
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
        let raw = self.load_raw_samples(query.start, query.end, query.battery_id.as_deref())?;
        history_response(query, &raw)
    }

    /// Loads raw, ordered samples in an inclusive UTC range for one or every
    /// battery. Shared by `history` and `day_usage_summary` so both read the
    /// same immutable rows the same way.
    fn load_raw_samples(
        &self,
        start: OffsetDateTime,
        end: OffsetDateTime,
        battery_id: Option<&str>,
    ) -> Result<Vec<RawHistorySample>, StorageError> {
        let start = format_utc(start)?;
        let end = format_utc(end)?;
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
        statement
            .query_map(params![start, end, battery_id], raw_history_sample)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Summarizes observed usage for one local calendar day, expressed as a
    /// UTC instant range resolved by the caller from timezone/DST-aware local
    /// midnight boundaries.
    ///
    /// This reuses the same gap-respecting interval and coverage arithmetic as
    /// `history` (`history_gaps`, `union_observed_duration`) rather than
    /// re-deriving continuity rules. It never interpolates across a reboot,
    /// suspend, or sampling-gap boundary, and never fabricates a value the
    /// samples do not support.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid range or unreadable/malformed rows.
    pub fn day_usage_summary(
        &self,
        query: &DayUsageQuery,
    ) -> Result<DayUsageSummary, StorageError> {
        if query.start > query.end {
            return Err(StorageError::InvalidHistoryQuery(
                "start must not be after end".to_owned(),
            ));
        }
        let elapsed_seconds = (query.end - query.start).as_seconds_f64().max(0.0);
        let raw = self.load_raw_samples(query.start, query.end, query.battery_id.as_deref())?;
        let sample_count = raw.len();
        let gaps = history_gaps(&raw);
        let summary = history_summary(&raw, &gaps);
        let coverage_ratio = match (summary.observed_duration_seconds, elapsed_seconds) {
            (Some(observed), elapsed) if elapsed > 0.0 => Some((observed / elapsed).min(1.0)),
            _ => None,
        };

        // Percentage endpoints and direction-specific average power are only
        // ever computed for one physical battery: interleaving raw rows from
        // distinct batteries by array position would silently mix devices.
        let single_battery = query.battery_id.is_some();
        let valid_percentages = raw
            .iter()
            .filter_map(|sample| sample.sample.metrics.percentage.value)
            .collect::<Vec<_>>();
        let (start_percentage, end_percentage) = if single_battery {
            (
                valid_percentages.first().copied(),
                valid_percentages.last().copied(),
            )
        } else {
            (None, None)
        };
        let percentage_change = match (start_percentage, end_percentage) {
            (Some(start), Some(end)) => Some(end - start),
            _ => None,
        };
        let representative_full_energy_wh = raw
            .iter()
            .filter_map(|sample| sample.sample.metrics.energy_full_wh.value)
            .fold(None, |max, value| {
                Some(max.map_or(value, |max: f64| max.max(value)))
            });
        let (average_discharge_power_watts, average_charge_power_watts) = if single_battery {
            (
                average_power_for_state(&raw, SampleState::Discharging),
                average_power_for_state(&raw, SampleState::Charging),
            )
        } else {
            (None, None)
        };

        let insufficiency_reason = if sample_count == 0 {
            Some(DayInsufficiencyReason::NoRecording)
        } else if sample_count < MIN_DAY_USAGE_SAMPLES
            || summary
                .observed_duration_seconds
                .is_none_or(|observed| observed < MIN_DAY_USAGE_COVERED_SECONDS)
        {
            Some(DayInsufficiencyReason::TooFewSamples)
        } else {
            None
        };
        let evidence = if insufficiency_reason.is_none() {
            DayEvidence::Sufficient
        } else {
            DayEvidence::Insufficient
        };

        Ok(DayUsageSummary {
            battery_id: query.battery_id.clone(),
            sample_count,
            elapsed_seconds,
            observed_duration_seconds: summary.observed_duration_seconds,
            coverage_ratio,
            start_percentage,
            end_percentage,
            percentage_change,
            energy_change_wh: summary.observed_energy_wh,
            representative_full_energy_wh,
            average_discharge_power_watts,
            average_charge_power_watts,
            evidence,
            insufficiency_reason,
        })
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

    /// Groups every completed discharge session into a starting-charge band
    /// and reports the observed duration distribution per band.
    ///
    /// Supplying `query.battery_id: None` pools completed discharge sessions
    /// across every physical battery into one set of duration statistics.
    /// This is valid because a session's own observed duration is a single
    /// scalar (unlike raw percentage or power series, which are never
    /// combined across distinct batteries elsewhere in this module): "how
    /// long a full-charge run lasted" is meaningful evidence regardless of
    /// which physical battery produced it.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid session query or unreadable derived rows.
    pub fn discharge_duration_by_starting_band(
        &self,
        query: &SessionQuery,
    ) -> Result<DischargeDurationByStartingBand, StorageError> {
        let sessions = self.sessions(query)?;
        Ok(discharge_duration_by_starting_band_from_sessions(
            &sessions,
            query.battery_id.clone(),
        ))
    }

    /// Looks up the observed percent-per-hour rate of comparable historical
    /// sessions for the live runtime forecast: sessions that both match
    /// `kind` (charging or discharging) and themselves started in the same
    /// `STARTING_CHARGE_BANDS` band as `current_percentage`. See
    /// [`HistoricalRateBand`] for why bands are matched exactly rather than
    /// interpolated. Returns `None` only when `current_percentage` or `kind`
    /// is not a value this forecast supports (out of `0.0..=100.0`, or
    /// neither charging nor discharging); the caller is expected to have
    /// already validated both before calling.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid session query or unreadable derived rows.
    pub fn historical_rate_by_current_percentage(
        &self,
        query: &SessionQuery,
        kind: BatterySessionKind,
        current_percentage: f64,
    ) -> Result<Option<HistoricalRateBand>, StorageError> {
        let sessions = self.sessions(query)?;
        Ok(historical_rate_by_current_percentage_from_sessions(
            &sessions,
            kind,
            current_percentage,
        ))
    }

    /// Reads a short, recent, same-boot trend in observed percentage for one
    /// physical battery, to blend into the live runtime forecast alongside
    /// the historical band rate. See [`recent_rate_from_raw_samples`] for the
    /// evidence policy (contiguity, minimum window) applied.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying sample query cannot be read.
    pub fn recent_rate_percent_per_hour(
        &self,
        battery_id: &str,
        now: OffsetDateTime,
        lookback: time::Duration,
    ) -> Result<Option<RecentRateEvidence>, StorageError> {
        let raw = self.load_raw_samples(now - lookback, now, Some(battery_id))?;
        Ok(recent_rate_from_raw_samples(&raw))
    }

    #[cfg(test)]
    fn connection(&self) -> &Connection {
        &self.connection
    }
}

/// Pure, deterministic bucketing/statistics logic shared by
/// `Storage::discharge_duration_by_starting_band` and its fixture-based unit
/// tests. Only completed discharge sessions with a known starting percentage
/// and a positive observed duration are considered; every other session
/// (charging, incomplete, unknown state, or missing endpoints) is silently
/// excluded rather than guessed at.
fn discharge_duration_by_starting_band_from_sessions(
    sessions: &[BatterySession],
    battery_id: Option<String>,
) -> DischargeDurationByStartingBand {
    // (duration_minutes, fully_drained) per band, in `STARTING_CHARGE_BANDS` order.
    let mut band_sessions: Vec<Vec<(f64, bool)>> =
        STARTING_CHARGE_BANDS.iter().map(|_| Vec::new()).collect();
    let mut considered = 0_u64;
    let mut earliest_started_at: Option<&str> = None;
    let mut latest_ended_at: Option<&str> = None;

    for session in sessions {
        if session.kind != BatterySessionKind::Discharging || !session.complete {
            continue;
        }
        let (Some(start_percentage), Some(duration_seconds)) =
            (session.start_percentage, session.observed_duration_seconds)
        else {
            continue;
        };
        if duration_seconds <= 0.0 {
            continue;
        }
        let Some(band_index) = STARTING_CHARGE_BANDS.iter().position(|&(low, high)| {
            start_percentage >= low && (start_percentage < high || high >= 100.0)
        }) else {
            continue;
        };

        considered += 1;
        earliest_started_at = Some(match earliest_started_at {
            Some(existing) if existing <= session.started_at.as_str() => existing,
            _ => session.started_at.as_str(),
        });
        latest_ended_at = Some(match latest_ended_at {
            Some(existing) if existing >= session.ended_at.as_str() => existing,
            _ => session.ended_at.as_str(),
        });

        let fully_drained = session
            .end_percentage
            .is_some_and(|end| end <= FULLY_DRAINED_MAX_PERCENT);
        band_sessions[band_index].push((duration_seconds / 60.0, fully_drained));
    }

    let bands = STARTING_CHARGE_BANDS
        .iter()
        .zip(band_sessions)
        .map(|(&(low, high), durations)| {
            let all_minutes: Vec<f64> = durations.iter().map(|&(minutes, _)| minutes).collect();
            let drained_minutes: Vec<f64> = durations
                .iter()
                .filter_map(|&(minutes, drained)| drained.then_some(minutes))
                .collect();
            StartingChargeBandSummary {
                band_start_percent: low,
                band_end_percent: high,
                is_full_charge_band: (low - FULL_CHARGE_BAND_MIN_PERCENT).abs() < f64::EPSILON,
                all_sessions: duration_stats_minutes(&all_minutes),
                fully_drained: duration_stats_minutes(&drained_minutes),
            }
        })
        .collect();

    DischargeDurationByStartingBand {
        battery_id,
        bands,
        session_count: considered,
        earliest_session_started_at: earliest_started_at.map(str::to_owned),
        latest_session_ended_at: latest_ended_at.map(str::to_owned),
    }
}

/// Computes count/average/median/min/max over a non-empty set of minute
/// durations. Returns `None` for an empty set rather than a fabricated zero.
#[allow(clippy::cast_precision_loss)]
fn duration_stats_minutes(minutes: &[f64]) -> Option<DurationStatsMinutes> {
    if minutes.is_empty() {
        return None;
    }
    let mut sorted = minutes.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).expect("durations are finite"));
    let count = sorted.len();
    let sum: f64 = sorted.iter().sum();
    let median = if count % 2 == 1 {
        sorted[count / 2]
    } else {
        f64::midpoint(sorted[count / 2 - 1], sorted[count / 2])
    };
    Some(DurationStatsMinutes {
        count: u64::try_from(count).unwrap_or(u64::MAX),
        average_minutes: sum / count as f64,
        median_minutes: median,
        min_minutes: sorted[0],
        max_minutes: sorted[count - 1],
    })
}

/// Returns the `STARTING_CHARGE_BANDS` band containing `percent`, using the
/// same membership rule already relied on by
/// `discharge_duration_by_starting_band_from_sessions` (`low <= percent <
/// high`, with the top band also including exactly 100).
fn band_containing(percent: f64) -> Option<(f64, f64)> {
    STARTING_CHARGE_BANDS
        .iter()
        .copied()
        .find(|&(low, high)| percent >= low && (percent < high || high >= 100.0))
}

/// The observed rate for one completed session, in percent per hour of
/// observed duration. `None` when the session lacks a usable start/end
/// percentage pair, a positive observed duration, or any actual observed
/// percentage change — a zero-change session says nothing about rate and
/// must never be averaged in as a zero.
fn session_rate_percent_per_hour(session: &BatterySession) -> Option<f64> {
    let (Some(start), Some(end), Some(duration_seconds)) = (
        session.start_percentage,
        session.end_percentage,
        session.observed_duration_seconds,
    ) else {
        return None;
    };
    if duration_seconds <= 0.0 {
        return None;
    }
    let delta = (end - start).abs();
    if delta <= 0.0 {
        return None;
    }
    Some(delta / (duration_seconds / 3600.0))
}

/// Computes count/average/median/min/max over a non-empty set of
/// percent-per-hour rates. Returns `None` for an empty set rather than a
/// fabricated zero. Mirrors `duration_stats_minutes` for the rate domain.
#[allow(clippy::cast_precision_loss)]
fn rate_stats_percent_per_hour(rates: &[f64]) -> Option<RatePercentPerHourStats> {
    if rates.is_empty() {
        return None;
    }
    let mut sorted = rates.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).expect("rates are finite"));
    let count = sorted.len();
    let sum: f64 = sorted.iter().sum();
    let median = if count % 2 == 1 {
        sorted[count / 2]
    } else {
        f64::midpoint(sorted[count / 2 - 1], sorted[count / 2])
    };
    Some(RatePercentPerHourStats {
        count: u64::try_from(count).unwrap_or(u64::MAX),
        average_percent_per_hour: sum / count as f64,
        median_percent_per_hour: median,
        min_percent_per_hour: sorted[0],
        max_percent_per_hour: sorted[count - 1],
    })
}

/// Pure, deterministic evidence lookup shared by
/// `Storage::historical_rate_by_current_percentage` and its fixture-based
/// unit tests. Only completed sessions of the requested `kind` whose own
/// starting percentage falls in the same band as `current_percentage` are
/// considered — see [`HistoricalRateBand`] for why bands are not blended
/// across their boundaries.
fn historical_rate_by_current_percentage_from_sessions(
    sessions: &[BatterySession],
    kind: BatterySessionKind,
    current_percentage: f64,
) -> Option<HistoricalRateBand> {
    if !current_percentage.is_finite() || !(0.0..=100.0).contains(&current_percentage) {
        return None;
    }
    if !matches!(
        kind,
        BatterySessionKind::Charging | BatterySessionKind::Discharging
    ) {
        return None;
    }
    let (low, high) = band_containing(current_percentage)?;
    let rates = sessions
        .iter()
        .filter(|session| session.kind == kind && session.complete)
        .filter(|session| {
            session
                .start_percentage
                .is_some_and(|start| start >= low && (start < high || high >= 100.0))
        })
        .filter_map(session_rate_percent_per_hour)
        .collect::<Vec<_>>();
    Some(HistoricalRateBand {
        band_start_percent: low,
        band_end_percent: high,
        stats: rate_stats_percent_per_hour(&rates),
    })
}

/// Minimum recent window, in minutes, before a live trend is treated as
/// meaningful evidence at all. The recorder writes roughly once a minute
/// (see `MAX_CONTIGUOUS_SAMPLE_SECONDS`), so a shorter window is one or two
/// samples of ordinary reporting jitter, not an observed trend.
const MIN_LIVE_RATE_WINDOW_MINUTES: f64 = 5.0;

/// Pure, deterministic trend extraction shared by
/// `Storage::recent_rate_percent_per_hour` and its fixture-based unit tests.
/// Uses only the oldest and newest sample of the supplied window, but only
/// once the whole window is confirmed to be one contiguous, same-boot run:
/// any internal gap (suspend, reboot, a stopped recorder) — as detected by
/// the same `history_gaps` logic charts use to break a line — makes the
/// window untrustworthy as a single trend, so it is rejected rather than
/// spliced across the discontinuity.
fn recent_rate_from_raw_samples(raw: &[RawHistorySample]) -> Option<RecentRateEvidence> {
    if raw.len() < 2 {
        return None;
    }
    if !history_gaps(raw).is_empty() {
        return None;
    }
    let first = raw.first()?;
    let last = raw.last()?;
    if first.sample.boot_id != last.sample.boot_id {
        return None;
    }
    let (Some(first_percentage), Some(last_percentage)) = (
        first.sample.metrics.percentage.value,
        last.sample.metrics.percentage.value,
    ) else {
        return None;
    };
    let window_seconds = (last.recorded_at - first.recorded_at).as_seconds_f64();
    if !window_seconds.is_finite() || window_seconds <= 0.0 {
        return None;
    }
    let window_minutes = window_seconds / 60.0;
    if window_minutes < MIN_LIVE_RATE_WINDOW_MINUTES {
        return None;
    }
    let delta = (last_percentage - first_percentage).abs();
    if delta <= 0.0 {
        return None;
    }
    Some(RecentRateEvidence {
        rate_percent_per_hour: delta / (window_seconds / 3600.0),
        window_minutes,
        sample_count: raw.len(),
    })
}

/// Minimum recent window, in minutes, before the live trend receives any
/// blend weight at all — see `MIN_LIVE_RATE_WINDOW_MINUTES`, which this
/// mirrors so a live trend either counts as evidence or does not; there is
/// no third, half-trusted state.
const LIVE_RATE_MIN_BLEND_MINUTES: f64 = MIN_LIVE_RATE_WINDOW_MINUTES;
/// Recent-window length, in minutes, at which the live trend reaches its
/// maximum blend weight (`LIVE_RATE_MAX_WEIGHT`).
const LIVE_RATE_FULL_BLEND_MINUTES: f64 = 60.0;
/// Upper bound on how much weight the live trend can ever receive. A single
/// noisy live reading must never be allowed to dominate a projection built
/// from many historical sessions: the historical average always keeps at
/// least 60% of the weight, so the number shown does not swing wildly
/// across manual refreshes just because the last few minutes looked
/// unusual, while the live signal still nudges the projection when it has
/// been observed for a while.
const LIVE_RATE_MAX_WEIGHT: f64 = 0.4;

/// Blends the stable historical band rate with a short live trend.
///
/// The live signal is ignored entirely below `LIVE_RATE_MIN_BLEND_MINUTES`
/// of confirmed contiguous coverage, then ramps linearly in weight up to
/// `LIVE_RATE_FULL_BLEND_MINUTES`, capped at `LIVE_RATE_MAX_WEIGHT` beyond
/// that. This lets recent behavior move the projection — for example a
/// laptop working noticeably harder right now than its historical average —
/// without ever letting one recent window outweigh the accumulated
/// historical evidence, even when the two disagree sharply.
#[must_use]
pub fn blend_rate_percent_per_hour(
    historical_average_percent_per_hour: f64,
    recent: Option<RecentRateEvidence>,
) -> f64 {
    let Some(recent) = recent else {
        return historical_average_percent_per_hour;
    };
    if recent.window_minutes < LIVE_RATE_MIN_BLEND_MINUTES {
        return historical_average_percent_per_hour;
    }
    let progress = ((recent.window_minutes - LIVE_RATE_MIN_BLEND_MINUTES)
        / (LIVE_RATE_FULL_BLEND_MINUTES - LIVE_RATE_MIN_BLEND_MINUTES))
        .clamp(0.0, 1.0);
    let live_weight = progress * LIVE_RATE_MAX_WEIGHT;
    historical_average_percent_per_hour.mul_add(
        1.0 - live_weight,
        recent.rate_percent_per_hour * live_weight,
    )
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

/// Mean recorded power over samples observed in one state. This intentionally
/// averages independent instantaneous readings rather than integrating a
/// rate across time, so it stays valid even when samples for that state are
/// separated by a gap: no interval is assumed between them.
fn average_power_for_state(raw: &[RawHistorySample], state: SampleState) -> Option<f64> {
    let values = raw
        .iter()
        .filter(|sample| sample.sample.state == state)
        .filter_map(|sample| sample.sample.metrics.power_watts.value)
        .map(f64::abs)
        .collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let count = values.len() as f64;
    Some(values.into_iter().sum::<f64>() / count)
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
#[allow(clippy::cast_precision_loss, clippy::float_cmp)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        BatterySession, BatterySessionKind, DayEvidence, DayInsufficiencyReason, DayUsageQuery,
        HistoryFreshness, HistoryGapReason, HistoryQuery, HistoryTimelineItem, InsertOutcome,
        LIVE_RATE_FULL_BLEND_MINUTES, LIVE_RATE_MAX_WEIGHT, LIVE_RATE_MIN_BLEND_MINUTES,
        MAX_CONTIGUOUS_SAMPLE_SECONDS, MetricSource, NewBatterySample, RecentRateEvidence,
        SampleMetric, SampleMetrics, SampleState, SessionAggregationPeriod, SessionInterruptReason,
        SessionQuery, Storage, StorageError, blend_rate_percent_per_hour,
        database_path_from_data_home, discharge_duration_by_starting_band_from_sessions,
        historical_rate_by_current_percentage_from_sessions, session_bucket,
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

    fn day_range() -> (OffsetDateTime, OffsetDateTime) {
        (
            datetime!(2026-08-23 00:00 UTC),
            datetime!(2026-08-24 00:00 UTC),
        )
    }

    fn day_sample(
        minutes: i64,
        percentage: f64,
        power_watts: f64,
        state: SampleState,
    ) -> NewBatterySample {
        let mut observation = sample_at(minutes, Some(percentage));
        observation.state = state;
        observation.metrics.power_watts = SampleMetric {
            value: Some(power_watts),
            source: MetricSource::Sysfs,
        };
        observation
    }

    #[test]
    fn day_usage_summary_reports_no_recording_for_an_empty_day() {
        let root = temporary_path("day-usage-empty");
        let path = database_path_from_data_home(&root);
        let storage = Storage::open_at(&path).expect("database opens");
        let (start, end) = day_range();

        let summary = storage
            .day_usage_summary(&DayUsageQuery {
                battery_id: Some("BAT0".to_owned()),
                start,
                end,
            })
            .expect("day usage summary reads");

        assert_eq!(summary.sample_count, 0);
        assert_eq!(summary.evidence, DayEvidence::Insufficient);
        assert_eq!(
            summary.insufficiency_reason,
            Some(DayInsufficiencyReason::NoRecording)
        );
        assert_eq!(summary.percentage_change, None);
        assert_eq!(summary.energy_change_wh, None);
        assert_eq!(summary.observed_duration_seconds, None);
        assert_eq!(summary.coverage_ratio, None);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn day_usage_summary_reports_a_day_that_has_not_started_yet_as_no_recording() {
        let root = temporary_path("day-usage-not-started");
        let path = database_path_from_data_home(&root);
        let storage = Storage::open_at(&path).expect("database opens");
        let midnight = datetime!(2026-08-23 00:00 UTC);

        let summary = storage
            .day_usage_summary(&DayUsageQuery {
                battery_id: Some("BAT0".to_owned()),
                start: midnight,
                end: midnight,
            })
            .expect("day usage summary reads");

        assert_eq!(summary.elapsed_seconds, 0.0);
        assert_eq!(summary.sample_count, 0);
        assert_eq!(summary.evidence, DayEvidence::Insufficient);
        assert_eq!(
            summary.insufficiency_reason,
            Some(DayInsufficiencyReason::NoRecording)
        );
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn day_usage_summary_reports_too_few_samples_below_the_evidence_policy() {
        let root = temporary_path("day-usage-too-few");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        for minute in 0_i64..3 {
            let observation = day_sample(
                minute,
                80.0 - minute as f64,
                -10.0,
                SampleState::Discharging,
            );
            storage.insert_sample(&observation).expect("sample inserts");
        }

        let (start, end) = day_range();
        let summary = storage
            .day_usage_summary(&DayUsageQuery {
                battery_id: Some("BAT0".to_owned()),
                start,
                end,
            })
            .expect("day usage summary reads");

        assert_eq!(summary.sample_count, 3);
        assert_eq!(summary.evidence, DayEvidence::Insufficient);
        assert_eq!(
            summary.insufficiency_reason,
            Some(DayInsufficiencyReason::TooFewSamples)
        );
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn day_usage_summary_computes_change_and_average_power_with_sufficient_evidence() {
        let root = temporary_path("day-usage-sufficient");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        for minute in 0_i64..=11 {
            let observation = day_sample(
                minute,
                80.0 - minute as f64,
                -10.0,
                SampleState::Discharging,
            );
            storage.insert_sample(&observation).expect("sample inserts");
        }

        let (start, end) = day_range();
        let summary = storage
            .day_usage_summary(&DayUsageQuery {
                battery_id: Some("BAT0".to_owned()),
                start,
                end,
            })
            .expect("day usage summary reads");

        assert_eq!(summary.sample_count, 12);
        assert_eq!(summary.evidence, DayEvidence::Sufficient);
        assert_eq!(summary.insufficiency_reason, None);
        assert_eq!(summary.start_percentage, Some(80.0));
        assert_eq!(summary.end_percentage, Some(69.0));
        assert_eq!(summary.percentage_change, Some(-11.0));
        assert_eq!(summary.average_discharge_power_watts, Some(10.0));
        assert_eq!(summary.average_charge_power_watts, None);
        let observed = summary
            .observed_duration_seconds
            .expect("contiguous coverage exists");
        assert!((observed - 660.0).abs() < 0.01);
        let energy = summary.energy_change_wh.expect("constant power integrates");
        assert!((energy - (-10.0 * 660.0 / 3600.0)).abs() < 0.01);
        let ratio = summary.coverage_ratio.expect("elapsed time is known");
        assert!(ratio > 0.0 && ratio < 1.0);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn day_usage_summary_never_bridges_a_reboot_gap() {
        let root = temporary_path("day-usage-reboot-gap");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        for minute in 0_i64..6 {
            let observation = day_sample(
                minute,
                80.0 - minute as f64,
                -10.0,
                SampleState::Discharging,
            );
            storage.insert_sample(&observation).expect("sample inserts");
        }
        for minute in 60_i64..66 {
            let mut observation = day_sample(
                minute,
                80.0 - minute as f64,
                -10.0,
                SampleState::Discharging,
            );
            observation.boot_id = "99999999-8888-7777-6666-555555555555".to_owned();
            observation.boot_seconds = (minute - 60) as f64 * 60.0;
            storage.insert_sample(&observation).expect("sample inserts");
        }

        let (start, end) = day_range();
        let summary = storage
            .day_usage_summary(&DayUsageQuery {
                battery_id: Some("BAT0".to_owned()),
                start,
                end,
            })
            .expect("day usage summary reads");

        assert_eq!(summary.sample_count, 12);
        let observed = summary
            .observed_duration_seconds
            .expect("two contiguous runs are observed");
        // Only the two five-minute contiguous runs count; the reboot boundary
        // between them must never be counted as observed or bridged.
        assert!((observed - 600.0).abs() < 0.01);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn day_usage_summary_omits_percentage_and_direction_power_for_multi_battery_scope() {
        let root = temporary_path("day-usage-multi-battery");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        for minute in 0_i64..12 {
            let observation = day_sample(
                minute,
                80.0 - minute as f64,
                -10.0,
                SampleState::Discharging,
            );
            storage.insert_sample(&observation).expect("sample inserts");
            let mut other = day_sample(minute, 50.0 + minute as f64, 8.0, SampleState::Charging);
            other.battery_id = "BAT1".to_owned();
            storage.insert_sample(&other).expect("sample inserts");
        }

        let (start, end) = day_range();
        let summary = storage
            .day_usage_summary(&DayUsageQuery {
                battery_id: None,
                start,
                end,
            })
            .expect("day usage summary reads");

        assert_eq!(summary.battery_id, None);
        assert_eq!(summary.sample_count, 24);
        assert_eq!(summary.evidence, DayEvidence::Sufficient);
        assert_eq!(summary.start_percentage, None);
        assert_eq!(summary.end_percentage, None);
        assert_eq!(summary.percentage_change, None);
        assert_eq!(summary.average_discharge_power_watts, None);
        assert_eq!(summary.average_charge_power_watts, None);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    /// Builds a minimal completed discharge session fixture for the
    /// starting-charge-band bucketing tests. `started_at`/`ended_at` only
    /// need to sort correctly against one another; they are not parsed as
    /// timestamps by the pure bucketing function under test.
    fn discharge_session(
        battery_id: &str,
        started_at: &str,
        ended_at: &str,
        start_percentage: f64,
        end_percentage: f64,
        duration_minutes: f64,
    ) -> BatterySession {
        BatterySession {
            battery_id: battery_id.to_owned(),
            kind: BatterySessionKind::Discharging,
            started_at: started_at.to_owned(),
            ended_at: ended_at.to_owned(),
            sample_count: 10,
            observed_duration_seconds: Some(duration_minutes * 60.0),
            start_percentage: Some(start_percentage),
            end_percentage: Some(end_percentage),
            start_energy_wh: None,
            end_energy_wh: None,
            average_power_watts: None,
            complete: true,
            interrupt_reason: SessionInterruptReason::StateChanged,
        }
    }

    #[test]
    fn starting_charge_bands_report_no_evidence_for_an_empty_session_list() {
        let result = discharge_duration_by_starting_band_from_sessions(&[], None);

        assert_eq!(result.session_count, 0);
        assert_eq!(result.earliest_session_started_at, None);
        assert_eq!(result.latest_session_ended_at, None);
        assert_eq!(result.bands.len(), 6);
        for band in &result.bands {
            assert_eq!(band.all_sessions, None);
            assert_eq!(band.fully_drained, None);
        }
        let full_charge_band = result
            .bands
            .iter()
            .find(|band| band.is_full_charge_band)
            .expect("a full-charge band exists");
        assert_eq!(full_charge_band.band_start_percent, 95.0);
        assert_eq!(full_charge_band.band_end_percent, 100.0);
    }

    #[test]
    fn starting_charge_bands_ignore_incomplete_charging_and_unbounded_sessions() {
        let sessions = vec![
            // Charging sessions are never discharge evidence.
            BatterySession {
                kind: BatterySessionKind::Charging,
                ..discharge_session("BAT0", "t0", "t1", 20.0, 90.0, 60.0)
            },
            // Incomplete sessions have no trustworthy endpoint duration.
            BatterySession {
                complete: false,
                ..discharge_session("BAT0", "t0", "t1", 95.0, 10.0, 240.0)
            },
            // Missing start percentage cannot be banded.
            BatterySession {
                start_percentage: None,
                ..discharge_session("BAT0", "t0", "t1", 0.0, 5.0, 60.0)
            },
        ];

        let result = discharge_duration_by_starting_band_from_sessions(&sessions, None);

        assert_eq!(result.session_count, 0);
        assert!(result.bands.iter().all(|band| band.all_sessions.is_none()));
    }

    #[test]
    fn a_single_full_charge_discharge_populates_the_headline_band() {
        let sessions = vec![discharge_session("BAT0", "t0", "t1", 100.0, 8.0, 300.0)];

        let result = discharge_duration_by_starting_band_from_sessions(&sessions, None);

        assert_eq!(result.session_count, 1);
        let band = result
            .bands
            .iter()
            .find(|band| band.is_full_charge_band)
            .expect("full-charge band exists");
        let all_sessions = band.all_sessions.expect("one session recorded");
        assert_eq!(all_sessions.count, 1);
        assert!((all_sessions.average_minutes - 300.0).abs() < 1e-9);
        assert!((all_sessions.median_minutes - 300.0).abs() < 1e-9);
        assert!((all_sessions.min_minutes - 300.0).abs() < 1e-9);
        assert!((all_sessions.max_minutes - 300.0).abs() < 1e-9);
        let fully_drained = band.fully_drained.expect("session reached a low charge");
        assert_eq!(fully_drained.count, 1);
    }

    #[test]
    fn a_near_full_session_that_does_not_drain_low_is_excluded_from_the_headline_drain_stats() {
        // Starts at 97% (qualifies for the near-full band) but the user
        // plugs back in at 70%: this run says nothing about full battery
        // life and must not count as a "fully drained" headline sample,
        // even though it is still honest evidence for the band's general
        // duration distribution.
        let sessions = vec![discharge_session("BAT0", "t0", "t1", 97.0, 70.0, 45.0)];

        let result = discharge_duration_by_starting_band_from_sessions(&sessions, None);

        let band = result
            .bands
            .iter()
            .find(|band| band.is_full_charge_band)
            .expect("full-charge band exists");
        assert_eq!(
            band.all_sessions
                .expect("session recorded in the band")
                .count,
            1
        );
        assert_eq!(
            band.fully_drained, None,
            "a session that never reached a low charge must not count as a full discharge"
        );
    }

    #[test]
    fn multiple_sessions_in_one_band_compute_average_median_min_max() {
        let sessions = vec![
            discharge_session("BAT0", "t0", "t1", 96.0, 5.0, 200.0),
            discharge_session("BAT0", "t2", "t3", 98.0, 3.0, 260.0),
            discharge_session("BAT0", "t4", "t5", 100.0, 10.0, 300.0),
        ];

        let result = discharge_duration_by_starting_band_from_sessions(&sessions, None);

        let band = result
            .bands
            .iter()
            .find(|band| band.is_full_charge_band)
            .expect("full-charge band exists");
        let stats = band.fully_drained.expect("all three sessions drained low");
        assert_eq!(stats.count, 3);
        assert!((stats.average_minutes - 253.333_333_333_333_3).abs() < 1e-6);
        assert!((stats.median_minutes - 260.0).abs() < 1e-9);
        assert!((stats.min_minutes - 200.0).abs() < 1e-9);
        assert!((stats.max_minutes - 300.0).abs() < 1e-9);
    }

    #[test]
    fn sessions_are_distributed_across_multiple_bands() {
        let sessions = vec![
            discharge_session("BAT0", "t0", "t1", 97.0, 5.0, 300.0),
            discharge_session("BAT0", "t2", "t3", 85.0, 5.0, 200.0),
            discharge_session("BAT0", "t4", "t5", 50.0, 5.0, 90.0),
            discharge_session("BAT0", "t6", "t7", 15.0, 2.0, 20.0),
        ];

        let result = discharge_duration_by_starting_band_from_sessions(&sessions, None);

        assert_eq!(result.session_count, 4);
        let populated_bands = result
            .bands
            .iter()
            .filter(|band| band.all_sessions.is_some())
            .count();
        assert_eq!(populated_bands, 4);
        for (low, high) in [(95.0, 100.0), (80.0, 95.0), (40.0, 60.0), (0.0, 20.0)] {
            let band = result
                .bands
                .iter()
                .find(|band| band.band_start_percent == low && band.band_end_percent == high)
                .unwrap_or_else(|| panic!("band {low}-{high} exists"));
            assert_eq!(
                band.all_sessions
                    .expect("session recorded in this band")
                    .count,
                1
            );
        }
    }

    #[test]
    fn sessions_from_multiple_batteries_are_pooled_for_the_aggregate_scope() {
        let sessions = vec![
            discharge_session("BAT0", "t0", "t1", 96.0, 4.0, 240.0),
            discharge_session("BAT1", "t2", "t3", 99.0, 6.0, 260.0),
        ];

        let result = discharge_duration_by_starting_band_from_sessions(&sessions, None);

        assert_eq!(result.session_count, 2);
        let band = result
            .bands
            .iter()
            .find(|band| band.is_full_charge_band)
            .expect("full-charge band exists");
        assert_eq!(
            band.fully_drained
                .expect("both batteries' sessions drained low")
                .count,
            2
        );
    }

    #[test]
    fn the_battery_id_scope_is_reported_back_unchanged() {
        let sessions = vec![discharge_session("BAT0", "t0", "t1", 96.0, 4.0, 240.0)];

        let result =
            discharge_duration_by_starting_band_from_sessions(&sessions, Some("BAT0".to_owned()));

        assert_eq!(result.battery_id, Some("BAT0".to_owned()));
        assert_eq!(result.earliest_session_started_at, Some("t0".to_owned()));
        assert_eq!(result.latest_session_ended_at, Some("t1".to_owned()));
    }

    // -- Live runtime forecast: historical band rate --------------------

    #[test]
    fn runtime_forecast_reports_insufficient_evidence_for_an_empty_session_list() {
        let result = historical_rate_by_current_percentage_from_sessions(
            &[],
            BatterySessionKind::Discharging,
            55.0,
        )
        .expect("55% is a valid, in-range percentage");

        assert_eq!(result.band_start_percent, 40.0);
        assert_eq!(result.band_end_percent, 60.0);
        assert_eq!(result.stats, None);
    }

    #[test]
    fn runtime_forecast_rejects_percentages_and_kinds_it_cannot_forecast() {
        assert_eq!(
            historical_rate_by_current_percentage_from_sessions(
                &[],
                BatterySessionKind::Discharging,
                150.0,
            ),
            None
        );
        assert_eq!(
            historical_rate_by_current_percentage_from_sessions(
                &[],
                BatterySessionKind::Discharging,
                f64::NAN,
            ),
            None
        );
        assert_eq!(
            historical_rate_by_current_percentage_from_sessions(
                &[],
                BatterySessionKind::Full,
                55.0,
            ),
            None
        );
        assert_eq!(
            historical_rate_by_current_percentage_from_sessions(
                &[],
                BatterySessionKind::Unknown,
                55.0,
            ),
            None
        );
    }

    #[test]
    fn runtime_forecast_computes_a_clean_single_band_rate() {
        // Two comparable discharge sessions, both starting in the 40-60%
        // band: one loses 20 points in 2 hours (10%/h), the other loses 20
        // points in 1 hour (20%/h). Average and median must reflect exactly
        // those two observed rates, nothing extrapolated.
        let sessions = vec![
            discharge_session("BAT0", "t0", "t1", 55.0, 35.0, 120.0),
            discharge_session("BAT0", "t2", "t3", 50.0, 30.0, 60.0),
        ];

        let result = historical_rate_by_current_percentage_from_sessions(
            &sessions,
            BatterySessionKind::Discharging,
            48.0,
        )
        .expect("48% is a valid, in-range percentage");

        let stats = result.stats.expect("two comparable sessions were recorded");
        assert_eq!(stats.count, 2);
        assert!((stats.average_percent_per_hour - 15.0).abs() < 1e-9);
        assert!((stats.min_percent_per_hour - 10.0).abs() < 1e-9);
        assert!((stats.max_percent_per_hour - 20.0).abs() < 1e-9);
    }

    #[test]
    fn runtime_forecast_selects_the_band_boundary_exactly_rather_than_interpolating() {
        // 80.0% sits exactly on a band boundary. Membership is `low <= x <
        // high`, so 80.0% belongs to the 80-95 band, not the 60-80 band, and
        // a session that started at 75% (60-80 band) must not count as
        // evidence for it — no cross-band interpolation is performed.
        let sessions = vec![
            discharge_session("BAT0", "t0", "t1", 75.0, 65.0, 60.0),
            discharge_session("BAT0", "t2", "t3", 85.0, 80.0, 30.0),
        ];

        let result = historical_rate_by_current_percentage_from_sessions(
            &sessions,
            BatterySessionKind::Discharging,
            80.0,
        )
        .expect("80% is a valid, in-range percentage");

        assert_eq!(result.band_start_percent, 80.0);
        assert_eq!(result.band_end_percent, 95.0);
        let stats = result.stats.expect("the 85%-start session is in this band");
        assert_eq!(stats.count, 1);
        assert!((stats.average_percent_per_hour - 10.0).abs() < 1e-9);
    }

    #[test]
    fn runtime_forecast_pools_comparable_sessions_across_multiple_batteries() {
        let sessions = vec![
            discharge_session("BAT0", "t0", "t1", 30.0, 20.0, 60.0),
            discharge_session("BAT1", "t2", "t3", 35.0, 15.0, 120.0),
        ];

        let result = historical_rate_by_current_percentage_from_sessions(
            &sessions,
            BatterySessionKind::Discharging,
            32.0,
        )
        .expect("32% is a valid, in-range percentage");

        let stats = result.stats.expect("both batteries contributed evidence");
        assert_eq!(stats.count, 2);
    }

    #[test]
    fn runtime_forecast_charging_rate_uses_only_charging_sessions() {
        let sessions = vec![
            discharge_session("BAT0", "t0", "t1", 30.0, 20.0, 60.0),
            BatterySession {
                kind: BatterySessionKind::Charging,
                ..discharge_session("BAT0", "t2", "t3", 25.0, 55.0, 60.0)
            },
        ];

        let discharge_result = historical_rate_by_current_percentage_from_sessions(
            &sessions,
            BatterySessionKind::Discharging,
            28.0,
        )
        .expect("28% is a valid, in-range percentage");
        assert_eq!(
            discharge_result
                .stats
                .expect("the discharge session is evidence")
                .count,
            1
        );

        let charge_result = historical_rate_by_current_percentage_from_sessions(
            &sessions,
            BatterySessionKind::Charging,
            28.0,
        )
        .expect("28% is a valid, in-range percentage");
        let charge_stats = charge_result
            .stats
            .expect("the charging session is evidence");
        assert_eq!(charge_stats.count, 1);
        assert!((charge_stats.average_percent_per_hour - 30.0).abs() < 1e-9);
    }

    // -- Live runtime forecast: blending the live trend ------------------

    #[test]
    fn blend_ignores_a_missing_or_too_short_live_trend() {
        assert!((blend_rate_percent_per_hour(12.0, None) - 12.0).abs() < 1e-9);
        let too_short = RecentRateEvidence {
            rate_percent_per_hour: 40.0,
            window_minutes: 2.0,
            sample_count: 3,
        };
        assert!((blend_rate_percent_per_hour(12.0, Some(too_short)) - 12.0).abs() < 1e-9);
    }

    #[test]
    fn blend_never_lets_a_disagreeing_live_trend_dominate_the_historical_average() {
        // A long, confident live window (well past the full-blend point)
        // that disagrees sharply with history must still move the estimate
        // by no more than `LIVE_RATE_MAX_WEIGHT`, not replace it.
        let disagreeing = RecentRateEvidence {
            rate_percent_per_hour: 100.0,
            window_minutes: 240.0,
            sample_count: 50,
        };
        let blended = blend_rate_percent_per_hour(10.0, Some(disagreeing));
        let expected = 10.0f64.mul_add(1.0 - LIVE_RATE_MAX_WEIGHT, 100.0 * LIVE_RATE_MAX_WEIGHT);
        assert!((blended - expected).abs() < 1e-9);
        // The historical average still dominates the result.
        assert!(blended < 10.0 + (100.0 - 10.0) * LIVE_RATE_MAX_WEIGHT + 1e-9);
        assert!(blended > 10.0);
    }

    #[test]
    fn blend_ramps_weight_between_the_minimum_and_full_blend_windows() {
        let halfway = RecentRateEvidence {
            rate_percent_per_hour: 20.0,
            window_minutes: LIVE_RATE_MIN_BLEND_MINUTES
                + (LIVE_RATE_FULL_BLEND_MINUTES - LIVE_RATE_MIN_BLEND_MINUTES) / 2.0,
            sample_count: 10,
        };
        let blended = blend_rate_percent_per_hour(10.0, Some(halfway));
        let expected_weight = LIVE_RATE_MAX_WEIGHT / 2.0;
        let expected = 10.0f64.mul_add(1.0 - expected_weight, 20.0 * expected_weight);
        assert!((blended - expected).abs() < 1e-9);
    }

    // -- Live runtime forecast: recent same-boot trend from raw samples --

    #[test]
    fn recent_rate_percent_per_hour_reads_a_contiguous_recent_trend() {
        let root = temporary_path("runtime-forecast-recent-rate");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        for minute in 0_i64..=10 {
            let observation = day_sample(
                minute,
                60.0 - minute as f64,
                -12.0,
                SampleState::Discharging,
            );
            storage.insert_sample(&observation).expect("sample inserts");
        }

        let now = datetime!(2026-08-23 12:10 UTC);
        let recent = storage
            .recent_rate_percent_per_hour("BAT0", now, time::Duration::minutes(30))
            .expect("recent-rate query reads")
            .expect("a ten-minute contiguous window is sufficient evidence");

        // 10 percentage points lost over 10 minutes is 60%/h.
        assert!((recent.rate_percent_per_hour - 60.0).abs() < 1e-6);
        assert!((recent.window_minutes - 10.0).abs() < 1e-6);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn recent_rate_percent_per_hour_rejects_a_window_that_is_too_short() {
        let root = temporary_path("runtime-forecast-recent-rate-short");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        for minute in 0_i64..=2 {
            let observation = day_sample(
                minute,
                60.0 - minute as f64,
                -12.0,
                SampleState::Discharging,
            );
            storage.insert_sample(&observation).expect("sample inserts");
        }

        let now = datetime!(2026-08-23 12:02 UTC);
        let recent = storage
            .recent_rate_percent_per_hour("BAT0", now, time::Duration::minutes(30))
            .expect("recent-rate query reads");

        assert_eq!(recent, None);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }

    #[test]
    fn recent_rate_percent_per_hour_never_bridges_a_reboot_gap() {
        let root = temporary_path("runtime-forecast-recent-rate-reboot");
        let path = database_path_from_data_home(&root);
        let mut storage = Storage::open_at(&path).expect("database opens");
        for minute in 0_i64..6 {
            let observation = day_sample(
                minute,
                60.0 - minute as f64,
                -12.0,
                SampleState::Discharging,
            );
            storage.insert_sample(&observation).expect("sample inserts");
        }
        for minute in 6_i64..16 {
            let mut observation = day_sample(
                minute,
                60.0 - minute as f64,
                -12.0,
                SampleState::Discharging,
            );
            observation.boot_id = "99999999-8888-7777-6666-555555555555".to_owned();
            observation.boot_seconds = (minute - 6) as f64 * 60.0;
            storage.insert_sample(&observation).expect("sample inserts");
        }

        let now = datetime!(2026-08-23 12:15 UTC);
        let recent = storage
            .recent_rate_percent_per_hour("BAT0", now, time::Duration::minutes(30))
            .expect("recent-rate query reads");

        // The window spans the reboot boundary, so it must be rejected
        // rather than treated as one continuous trend.
        assert_eq!(recent, None);
        drop(storage);
        fs::remove_dir_all(root).expect("test directory is removable");
    }
}
