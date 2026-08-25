//! Native desktop entry point for Battery Dashboard.

#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use battery_dashboard_desktop::{
    anomalies, battery, export, health, power_profile, recorder_install,
    scheduler::{SchedulerStatus, SystemdUserScheduler},
    storage,
};
use chrono::{Datelike, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Returns current battery readings without persisting or altering system state.
#[tauri::command]
async fn get_battery_dashboard() -> battery::BatteryDashboardResponse {
    battery::read_dashboard().await
}

/// Returns bounded persisted history together with a clearly transient live point.
#[tauri::command]
async fn get_recent_battery_history(
    battery_id: Option<String>,
    range_hours: u16,
    max_points: usize,
) -> RecentBatteryHistoryResponse {
    let Some(range_hours) = supported_history_range(range_hours) else {
        return unavailable_history(battery_id, range_hours, "invalid-request");
    };
    if max_points == 0 {
        return unavailable_history(battery_id, range_hours, "invalid-request");
    }

    let end = OffsetDateTime::now_utc();
    let start = end - time::Duration::hours(i64::from(range_hours));
    let query = storage::HistoryQuery {
        start,
        end,
        battery_id: battery_id.clone(),
        max_points,
    };
    let recorder_state = recorder_state();
    let persisted = match storage::history_if_exists(&query) {
        Ok(history) => history,
        Err(_error) => {
            return unavailable_history(battery_id, range_hours, "database-unavailable");
        }
    };

    let dashboard = battery::read_dashboard().await;
    let mut mapped = persisted
        .as_ref()
        .map_or_else(MappedHistory::default, map_persisted_history);
    let transient = transient_live_point(&dashboard, battery_id.as_deref());
    if let Some(point) = transient {
        if !mapped
            .points
            .iter()
            .any(|existing| existing.recorded_at == point.recorded_at)
        {
            mapped.points.push(point);
        }
    }
    mapped
        .points
        .sort_by(|left, right| left.recorded_at.cmp(&right.recorded_at));

    let has_persisted = mapped.points.iter().any(|point| point.kind == "persisted");
    let has_transient = mapped.points.iter().any(|point| point.kind == "transient");
    let unavailable_reason = if recorder_state == "disabled" {
        Some("recorder-disabled")
    } else {
        (!has_persisted).then(|| recorder_unavailable_reason(recorder_state))
    };
    let availability = if has_persisted || has_transient {
        "available"
    } else {
        "unavailable"
    };
    let source = if has_persisted {
        "sqlite"
    } else if has_transient {
        "transient"
    } else {
        "unavailable"
    };
    let collected_at = dashboard.collected_at;
    let freshness = history_freshness(&mapped.points, end);
    let summary = history_summary_from_points(&mapped.points, &mapped.gaps);

    RecentBatteryHistoryResponse {
        schema_version: 1,
        availability,
        unavailable_reason,
        source,
        freshness,
        battery_id,
        range_hours,
        collected_at,
        points: mapped.points,
        gaps: mapped.gaps,
        summary,
    }
}

/// Returns conservative health values derived only from recorded `SQLite` samples.
///
/// Supplying no battery identifier is safe only when the history contains one
/// physical battery.  Capacity observations from multiple batteries are never
/// combined into a synthetic health value.  A missing metric remains `null` in
/// the response, and cycle count is returned only when the provider recorded a
/// hardware value.
#[tauri::command]
fn get_battery_health(battery_id: Option<String>) -> BatteryHealthResponse {
    if battery_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return unavailable_battery_health(battery_id, "invalid-request");
    }

    let query = all_history_query(battery_id.clone());
    let history = match storage::history_if_exists(&query) {
        Ok(Some(history)) => history,
        Ok(None) => {
            return unavailable_battery_health(
                battery_id,
                recorder_unavailable_reason(recorder_state()),
            );
        }
        Err(_) => return unavailable_battery_health(battery_id, "database-unavailable"),
    };
    let samples = history_samples(&history);
    if samples.is_empty() {
        return unavailable_battery_health(battery_id, "no-recorded-samples");
    }

    // An unfiltered query may contain more than one physical battery.  The
    // analysis layer intentionally refuses to combine those values, but the
    // command reports a dedicated reason so the UI can ask the user to select
    // one rather than presenting generic missing data.
    if battery_id.is_none() && distinct_battery_ids(&samples).len() > 1 {
        return unavailable_battery_health(None, "multiple-batteries");
    }

    let report = health::analyze(&samples);
    available_battery_health(report)
}

fn all_history_query(battery_id: Option<String>) -> storage::HistoryQuery {
    storage::HistoryQuery {
        start: OffsetDateTime::UNIX_EPOCH,
        end: OffsetDateTime::now_utc(),
        battery_id,
        // Health and export are reports over the immutable history, not chart
        // reads.  Keep every raw observation so daily medians and exported
        // records cannot be changed by visual downsampling.
        max_points: usize::MAX,
    }
}

fn history_samples(history: &storage::HistoryResponse) -> Vec<storage::HistorySample> {
    history
        .timeline
        .iter()
        .filter_map(|item| match item {
            storage::HistoryTimelineItem::Sample(sample) => Some(sample.as_ref().clone()),
            storage::HistoryTimelineItem::Gap(_) => None,
        })
        .collect()
}

fn distinct_battery_ids(samples: &[storage::HistorySample]) -> BTreeSet<&str> {
    samples
        .iter()
        .map(|sample| sample.battery_id.as_str())
        .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BatteryHealthResponse {
    schema_version: u8,
    availability: &'static str,
    unavailable_reason: Option<&'static str>,
    source: &'static str,
    battery_id: Option<String>,
    current_full_capacity_wh: Option<f64>,
    current_full_capacity_recorded_at: Option<String>,
    design_capacity_wh: Option<f64>,
    design_capacity_recorded_at: Option<String>,
    health_percentage: Option<f64>,
    health_recorded_at: Option<String>,
    hardware_cycle_count: Option<u64>,
    hardware_cycle_count_recorded_at: Option<String>,
    capacity_history: Vec<HealthCapacityPoint>,
    trend: &'static str,
    trend_slope_wh_per_day: Option<f64>,
    trend_upper_confidence_wh_per_day: Option<f64>,
    trend_insufficiency_reason: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthCapacityPoint {
    recorded_at: String,
    full_capacity_wh: f64,
}

fn unavailable_battery_health(
    battery_id: Option<String>,
    reason: &'static str,
) -> BatteryHealthResponse {
    BatteryHealthResponse {
        schema_version: 1,
        availability: "unavailable",
        unavailable_reason: Some(reason),
        source: "unavailable",
        battery_id,
        current_full_capacity_wh: None,
        current_full_capacity_recorded_at: None,
        design_capacity_wh: None,
        design_capacity_recorded_at: None,
        health_percentage: None,
        health_recorded_at: None,
        hardware_cycle_count: None,
        hardware_cycle_count_recorded_at: None,
        capacity_history: Vec::new(),
        trend: "insufficient",
        trend_slope_wh_per_day: None,
        trend_upper_confidence_wh_per_day: None,
        trend_insufficiency_reason: Some(reason),
    }
}

fn available_battery_health(report: health::BatteryHealthReport) -> BatteryHealthResponse {
    let trend = match &report.daily_degradation_trend {
        health::DailyDegradationTrend::Insufficient { .. } => "insufficient",
        health::DailyDegradationTrend::Inconclusive { .. } => "noisy",
        health::DailyDegradationTrend::Stable { .. } => "stable",
        health::DailyDegradationTrend::Degrading { .. } => "degrading",
    };
    let (trend_slope_wh_per_day, trend_upper_confidence_wh_per_day, trend_insufficiency_reason) =
        match report.daily_degradation_trend {
            health::DailyDegradationTrend::Insufficient { reason } => (
                None,
                None,
                Some(match reason {
                    health::TrendInsufficiency::TooFewDailyObservations => {
                        "too-few-daily-observations"
                    }
                    health::TrendInsufficiency::TooShortTimeSpan => "too-short-time-span",
                }),
            ),
            health::DailyDegradationTrend::Inconclusive {
                slope_wh_per_day,
                upper_confidence_wh_per_day,
            }
            | health::DailyDegradationTrend::Degrading {
                slope_wh_per_day,
                upper_confidence_wh_per_day,
            } => (
                Some(slope_wh_per_day),
                Some(upper_confidence_wh_per_day),
                None,
            ),
            health::DailyDegradationTrend::Stable { slope_wh_per_day } => {
                (Some(slope_wh_per_day), None, None)
            }
        };
    BatteryHealthResponse {
        schema_version: 1,
        availability: "available",
        unavailable_reason: None,
        source: "sqlite",
        battery_id: report.battery_id,
        current_full_capacity_wh: report
            .current_full_capacity
            .as_ref()
            .map(|capacity| capacity.watt_hours),
        current_full_capacity_recorded_at: report
            .current_full_capacity
            .as_ref()
            .map(|capacity| capacity.recorded_at.clone()),
        design_capacity_wh: report
            .current_design_capacity
            .as_ref()
            .map(|capacity| capacity.watt_hours),
        design_capacity_recorded_at: report
            .current_design_capacity
            .as_ref()
            .map(|capacity| capacity.recorded_at.clone()),
        health_percentage: report
            .health_percentage
            .as_ref()
            .map(|health| health.percent),
        health_recorded_at: report
            .health_percentage
            .as_ref()
            .map(|health| health.recorded_at.clone()),
        hardware_cycle_count: report
            .hardware_cycle_count
            .as_ref()
            .map(|cycle_count| cycle_count.count),
        hardware_cycle_count_recorded_at: report
            .hardware_cycle_count
            .as_ref()
            .map(|cycle_count| cycle_count.recorded_at.clone()),
        capacity_history: report
            .capacity_over_time
            .into_iter()
            .map(|point| HealthCapacityPoint {
                recorded_at: point.recorded_at,
                full_capacity_wh: point.full_capacity_wh,
            })
            .collect(),
        trend,
        trend_slope_wh_per_day,
        trend_upper_confidence_wh_per_day,
        trend_insufficiency_reason,
    }
}

/// Returns observational anomaly findings from immutable local samples.
///
/// The command never combines multiple physical batteries and never fills a
/// missing metric or interval.  A present but short history is reported as
/// `insufficient`, while a missing/unreadable backend is `unavailable`.
#[tauri::command]
fn get_battery_anomalies(
    battery_id: Option<String>,
    range_hours: Option<u16>,
) -> BatteryAnomaliesResponse {
    if battery_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return unavailable_battery_anomalies(battery_id, 24, "invalid-request");
    }
    let range_hours = range_hours.unwrap_or(24);
    if !(1..=720).contains(&range_hours) {
        return unavailable_battery_anomalies(battery_id, range_hours, "invalid-request");
    }
    let end = OffsetDateTime::now_utc();
    let start = end - time::Duration::hours(i64::from(range_hours));
    let query = storage::HistoryQuery {
        start,
        end,
        battery_id: battery_id.clone(),
        max_points: usize::MAX,
    };
    let history = match storage::history_if_exists(&query) {
        Ok(Some(history)) => history,
        Ok(None) => {
            return unavailable_battery_anomalies(
                battery_id,
                range_hours,
                recorder_unavailable_reason(recorder_state()),
            );
        }
        Err(_) => {
            return unavailable_battery_anomalies(battery_id, range_hours, "database-unavailable");
        }
    };
    let samples = history_samples(&history);
    if battery_id.is_none() && distinct_battery_ids(&samples).len() > 1 {
        return unavailable_battery_anomalies(battery_id, range_hours, "multiple-batteries");
    }
    anomaly_response(
        battery_id,
        range_hours,
        anomalies::analyze(&history.timeline),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BatteryAnomaliesResponse {
    schema_version: u8,
    availability: &'static str,
    unavailable_reason: Option<&'static str>,
    source: &'static str,
    battery_id: Option<String>,
    range_hours: u16,
    observed_samples: usize,
    power_samples: usize,
    discharge_intervals: usize,
    charging_transitions: usize,
    anomalies: Vec<anomalies::BatteryAnomaly>,
}

fn unavailable_battery_anomalies(
    battery_id: Option<String>,
    range_hours: u16,
    reason: &'static str,
) -> BatteryAnomaliesResponse {
    BatteryAnomaliesResponse {
        schema_version: 1,
        availability: "unavailable",
        unavailable_reason: Some(reason),
        source: "unavailable",
        battery_id,
        range_hours,
        observed_samples: 0,
        power_samples: 0,
        discharge_intervals: 0,
        charging_transitions: 0,
        anomalies: Vec::new(),
    }
}

fn anomaly_response(
    battery_id: Option<String>,
    range_hours: u16,
    report: anomalies::AnomalyReport,
) -> BatteryAnomaliesResponse {
    BatteryAnomaliesResponse {
        schema_version: 1,
        availability: report.availability,
        unavailable_reason: report
            .insufficiency_reason
            .map(anomalies::InsufficiencyReason::as_str),
        source: "sqlite",
        battery_id,
        range_hours,
        observed_samples: report.observed_samples,
        power_samples: report.power_samples,
        discharge_intervals: report.discharge_intervals,
        charging_transitions: report.charging_transitions,
        anomalies: report.anomalies,
    }
}

/// Reads the active local power profile without changing it.
#[tauri::command]
fn get_power_profile() -> power_profile::PowerProfileResponse {
    power_profile::get_profile()
}

/// Sets one explicitly allowlisted local power profile and verifies the result.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn set_power_profile(profile: String) -> power_profile::PowerProfileResponse {
    power_profile::set_profile(&profile)
}

/// Returns derived sessions and calendar buckets from immutable local samples.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn get_battery_session_history(
    battery_id: Option<String>,
    states: Option<Vec<String>>,
    start_date: Option<String>,
    end_date: Option<String>,
    timezone: String,
) -> SessionHistoryResponse {
    let Ok(timezone) = timezone.parse::<Tz>() else {
        return unavailable_session_history("invalid-request", &timezone);
    };
    let Ok((start, end)) = session_date_range(timezone, start_date.as_deref(), end_date.as_deref())
    else {
        return unavailable_session_history("invalid-request", timezone.name());
    };
    let Some(path) = storage::existing_database_path().ok().flatten() else {
        return unavailable_session_history(
            recorder_unavailable_reason(recorder_state()),
            timezone.name(),
        );
    };
    let Ok(storage) = storage::Storage::open_at(path) else {
        return unavailable_session_history("database-unavailable", timezone.name());
    };
    let query = storage::SessionQuery {
        start,
        end,
        battery_id,
    };
    let Ok(sessions) = storage.sessions(&query) else {
        return unavailable_session_history("database-unavailable", timezone.name());
    };
    let allowed = states
        .as_ref()
        .and_then(|values| normalize_session_states(values));
    if states.is_some() && allowed.is_none() {
        return unavailable_session_history("invalid-request", timezone.name());
    }
    let sessions = sessions
        .into_iter()
        .filter(|session| {
            allowed
                .as_ref()
                .is_none_or(|allowed| allowed.contains(session_kind(session.kind)))
        })
        .collect::<Vec<_>>();
    let daily = calendar_summaries(&sessions, timezone, "daily");
    let weekly = calendar_summaries(&sessions, timezone, "weekly");
    let monthly = calendar_summaries(&sessions, timezone, "monthly");
    SessionHistoryResponse {
        schema_version: 1,
        availability: "available",
        unavailable_reason: None,
        generated_at: format_timestamp(OffsetDateTime::now_utc()),
        timezone: timezone.name().to_owned(),
        sessions: sessions
            .into_iter()
            .enumerate()
            .map(|(index, session)| map_session(session, index))
            .collect(),
        daily,
        weekly,
        monthly,
    }
}

/// Rebuilds derived sessions; raw samples remain append-only and untouched.
#[tauri::command]
fn rebuild_battery_session_history() -> SessionRebuildResponse {
    let Some(path) = storage::existing_database_path().ok().flatten() else {
        return SessionRebuildResponse::unavailable(recorder_unavailable_reason(recorder_state()));
    };
    match storage::Storage::open_at(path).and_then(|mut storage| storage.rebuild_sessions()) {
        Ok(sessions_rebuilt) => SessionRebuildResponse {
            schema_version: 1,
            availability: "available",
            unavailable_reason: None,
            rebuilt_at: format_timestamp(OffsetDateTime::now_utc()),
            sessions_rebuilt: Some(sessions_rebuilt),
        },
        Err(_) => SessionRebuildResponse::unavailable("database-unavailable"),
    }
}

/// Compares today's and yesterday's observed usage from immutable local
/// samples, grouped by the same IANA/DST-aware local-calendar-day boundaries
/// already used for session history. Every derived figure stays absent,
/// never invented, when the underlying evidence is insufficient; "yesterday"
/// having no data at all (a fresh install) is reported as such, not as an
/// error.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn get_today_vs_yesterday_usage(
    battery_id: Option<String>,
    timezone: String,
) -> TodayVsYesterdayResponse {
    let Ok(parsed_timezone) = timezone.parse::<Tz>() else {
        return unavailable_today_vs_yesterday("invalid-request", &timezone);
    };
    if battery_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return unavailable_today_vs_yesterday("invalid-request", parsed_timezone.name());
    }
    let Some(path) = storage::existing_database_path().ok().flatten() else {
        return unavailable_today_vs_yesterday(
            recorder_unavailable_reason(recorder_state()),
            parsed_timezone.name(),
        );
    };
    let Ok(storage) = storage::Storage::open_at(path) else {
        return unavailable_today_vs_yesterday("database-unavailable", parsed_timezone.name());
    };

    let now = OffsetDateTime::now_utc();
    let today_date = Utc::now().with_timezone(&parsed_timezone).date_naive();
    let Some(yesterday_date) = today_date.pred_opt() else {
        return unavailable_today_vs_yesterday("invalid-request", parsed_timezone.name());
    };
    let (Ok((today_start, _)), Ok((yesterday_start, yesterday_end))) = (
        local_day_bounds(parsed_timezone, today_date),
        local_day_bounds(parsed_timezone, yesterday_date),
    ) else {
        return unavailable_today_vs_yesterday("invalid-request", parsed_timezone.name());
    };

    let (Some(today), Some(yesterday)) = (
        day_usage_dto(
            &storage,
            battery_id.as_deref(),
            today_date,
            today_start,
            now,
        ),
        day_usage_dto(
            &storage,
            battery_id.as_deref(),
            yesterday_date,
            yesterday_start,
            yesterday_end,
        ),
    ) else {
        return unavailable_today_vs_yesterday("database-unavailable", parsed_timezone.name());
    };

    TodayVsYesterdayResponse {
        schema_version: 1,
        availability: "available",
        unavailable_reason: None,
        generated_at: format_timestamp(now),
        timezone: parsed_timezone.name().to_owned(),
        battery_id,
        today,
        yesterday,
    }
}

/// Sessions below this count are shown but labelled a low-confidence, early
/// estimate rather than a settled figure; this is a wording threshold only,
/// never a cutoff that hides real recorded evidence.
const BATTERY_LIFE_LOW_CONFIDENCE_MAX_SESSIONS: u64 = 2;
/// At and above this count the headline is described as a moderately
/// confident estimate.
const BATTERY_LIFE_MODERATE_CONFIDENCE_MAX_SESSIONS: u64 = 5;

/// Reports observed discharge-duration evidence grouped by starting-charge
/// band, built only from completed discharge sessions recorded in local
/// `SQLite` history. The headline "on a full charge" figure uses only the band
/// of sessions that began at or above
/// `storage::FULL_CHARGE_BAND_MIN_PERCENT` and also drained down to or below
/// `storage::FULLY_DRAINED_MAX_PERCENT`; see those constants' documentation
/// for why those thresholds were chosen. No value here is ever extrapolated
/// beyond a session's own directly recorded start, end, and duration.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn get_battery_life_estimate(battery_id: Option<String>) -> BatteryLifeResponse {
    if battery_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return unavailable_battery_life("invalid-request");
    }
    let Some(path) = storage::existing_database_path().ok().flatten() else {
        return unavailable_battery_life(recorder_unavailable_reason(recorder_state()));
    };
    let Ok(storage) = storage::Storage::open_at(path) else {
        return unavailable_battery_life("database-unavailable");
    };

    let query = storage::SessionQuery {
        start: OffsetDateTime::UNIX_EPOCH,
        end: OffsetDateTime::now_utc(),
        battery_id: battery_id.clone(),
    };
    let Ok(summary) = storage.discharge_duration_by_starting_band(&query) else {
        return unavailable_battery_life("database-unavailable");
    };

    let bands = summary
        .bands
        .iter()
        .map(starting_charge_band_dto)
        .collect::<Vec<_>>();
    let headline_band = summary.bands.iter().find(|band| band.is_full_charge_band);
    let headline = battery_life_headline_dto(headline_band);

    BatteryLifeResponse {
        schema_version: 1,
        availability: "available",
        unavailable_reason: None,
        generated_at: format_timestamp(OffsetDateTime::now_utc()),
        battery_id,
        full_charge_min_percent: storage::FULL_CHARGE_BAND_MIN_PERCENT,
        fully_drained_max_percent: storage::FULLY_DRAINED_MAX_PERCENT,
        headline,
        bands,
        total_session_count: summary.session_count,
        earliest_session_started_at: summary.earliest_session_started_at,
        latest_session_ended_at: summary.latest_session_ended_at,
    }
}

fn unavailable_battery_life(reason: &'static str) -> BatteryLifeResponse {
    BatteryLifeResponse {
        schema_version: 1,
        availability: "unavailable",
        unavailable_reason: Some(reason),
        generated_at: None,
        battery_id: None,
        full_charge_min_percent: storage::FULL_CHARGE_BAND_MIN_PERCENT,
        fully_drained_max_percent: storage::FULLY_DRAINED_MAX_PERCENT,
        headline: BatteryLifeHeadlineDto {
            evidence: "insufficient",
            confidence: "none",
            session_count: 0,
            average_minutes: None,
            median_minutes: None,
            min_minutes: None,
            max_minutes: None,
        },
        bands: Vec::new(),
        total_session_count: 0,
        earliest_session_started_at: None,
        latest_session_ended_at: None,
    }
}

fn starting_charge_band_dto(band: &storage::StartingChargeBandSummary) -> StartingChargeBandDto {
    StartingChargeBandDto {
        band_start_percent: band.band_start_percent,
        band_end_percent: band.band_end_percent,
        is_full_charge_band: band.is_full_charge_band,
        all_sessions: band.all_sessions.map(duration_stats_dto),
        fully_drained: band.fully_drained.map(duration_stats_dto),
    }
}

fn duration_stats_dto(stats: storage::DurationStatsMinutes) -> DurationStatsDto {
    DurationStatsDto {
        count: stats.count,
        average_minutes: stats.average_minutes,
        median_minutes: stats.median_minutes,
        min_minutes: stats.min_minutes,
        max_minutes: stats.max_minutes,
    }
}

/// Builds the headline "on a full charge" figure strictly from the fully
/// drained subset of the near-full-start band: a run that starts near-full
/// but is interrupted early by plugging back in does not answer "how long
/// does my battery last".
fn battery_life_headline_dto(
    headline_band: Option<&storage::StartingChargeBandSummary>,
) -> BatteryLifeHeadlineDto {
    let Some(stats) = headline_band.and_then(|band| band.fully_drained) else {
        return BatteryLifeHeadlineDto {
            evidence: "insufficient",
            confidence: "none",
            session_count: 0,
            average_minutes: None,
            median_minutes: None,
            min_minutes: None,
            max_minutes: None,
        };
    };
    let confidence = if stats.count <= BATTERY_LIFE_LOW_CONFIDENCE_MAX_SESSIONS {
        "low"
    } else if stats.count <= BATTERY_LIFE_MODERATE_CONFIDENCE_MAX_SESSIONS {
        "moderate"
    } else {
        "high"
    };
    BatteryLifeHeadlineDto {
        evidence: "sufficient",
        confidence,
        session_count: stats.count,
        average_minutes: Some(stats.average_minutes),
        median_minutes: Some(stats.median_minutes),
        min_minutes: Some(stats.min_minutes),
        max_minutes: Some(stats.max_minutes),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BatteryLifeResponse {
    schema_version: u8,
    availability: &'static str,
    unavailable_reason: Option<&'static str>,
    generated_at: Option<String>,
    /// The requested scope: `None` means sessions were pooled across every
    /// physical battery.
    battery_id: Option<String>,
    full_charge_min_percent: f64,
    fully_drained_max_percent: f64,
    headline: BatteryLifeHeadlineDto,
    bands: Vec<StartingChargeBandDto>,
    total_session_count: u64,
    earliest_session_started_at: Option<String>,
    latest_session_ended_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BatteryLifeHeadlineDto {
    evidence: &'static str,
    confidence: &'static str,
    session_count: u64,
    average_minutes: Option<f64>,
    median_minutes: Option<f64>,
    min_minutes: Option<f64>,
    max_minutes: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartingChargeBandDto {
    band_start_percent: f64,
    band_end_percent: f64,
    is_full_charge_band: bool,
    all_sessions: Option<DurationStatsDto>,
    fully_drained: Option<DurationStatsDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DurationStatsDto {
    count: u64,
    average_minutes: f64,
    median_minutes: f64,
    min_minutes: f64,
    max_minutes: f64,
}

/// Which direction the live runtime forecast projects toward. Only
/// `charging` and `discharging` have anything to forecast: idle, full, and
/// unknown states are reported as `not-applicable` rather than a stale or
/// misleading number.
#[derive(Clone, Copy)]
enum RuntimeForecastKind {
    Charging,
    Discharging,
}

fn runtime_forecast_kind(state: &str) -> Option<RuntimeForecastKind> {
    match state {
        "charging" => Some(RuntimeForecastKind::Charging),
        "discharging" => Some(RuntimeForecastKind::Discharging),
        _ => None,
    }
}

/// Recent same-boot lookback window considered for the live blend in
/// `get_runtime_forecast`. Long enough to usually contain several
/// once-a-minute recorder samples (see
/// `storage::MIN_LIVE_RATE_WINDOW_MINUTES`'s reasoning), short enough that a
/// stale trend from much earlier in the session is never mistaken for
/// "right now".
const RUNTIME_FORECAST_LIVE_LOOKBACK_MINUTES: i64 = 30;

/// Reports a live, current-moment forecast of when the selected battery will
/// run out (while discharging) or finish charging (while charging), derived
/// from this machine's own recorded history rather than any `UPower`
/// estimate — `DEVELOPMENT_PLAN.md` section 11 keeps those two kinds of
/// estimate explicitly distinguished, and the frontend must label them
/// separately rather than blend them.
///
/// The projection combines two things, both built only from real recorded
/// evidence:
///
/// - the observed percent-per-hour rate of historical sessions that both
///   match the current direction (charging/discharging) and themselves
///   started in the same `storage::STARTING_CHARGE_BANDS` band as
///   `current_percentage` (see `storage::historical_rate_by_current_percentage`);
/// - a short recent same-boot trend from the last
///   `RUNTIME_FORECAST_LIVE_LOOKBACK_MINUTES` of recorded samples for the
///   selected battery, blended in with a bounded weight so it can never
///   dominate the historical baseline (see
///   `storage::blend_rate_percent_per_hour`).
///
/// The aggregate "all batteries" scope (`battery_id: None`) uses only the
/// historical band rate: combining raw percentage series across distinct
/// physical batteries by timestamp is avoided everywhere else in this
/// module (see `storage::DayUsageSummary`), so the live blend is skipped
/// there rather than guessed at.
///
/// Idle, full, or unknown states have nothing to forecast and are reported
/// as `not-applicable`. Insufficient historical evidence at the current
/// charge level is reported honestly (`evidence: "insufficient"`) instead of
/// a fabricated number.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn get_runtime_forecast(
    battery_id: Option<String>,
    state: String,
    current_percentage: f64,
) -> RuntimeForecastResponse {
    if battery_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return unavailable_runtime_forecast("invalid-request", battery_id, &state);
    }
    let Some(kind) = runtime_forecast_kind(&state) else {
        return not_applicable_runtime_forecast(battery_id, state);
    };
    if !current_percentage.is_finite() || !(0.0..=100.0).contains(&current_percentage) {
        return unavailable_runtime_forecast("invalid-request", battery_id, &state);
    }
    let Some(path) = storage::existing_database_path().ok().flatten() else {
        return unavailable_runtime_forecast(
            recorder_unavailable_reason(recorder_state()),
            battery_id,
            &state,
        );
    };
    let Ok(storage) = storage::Storage::open_at(path) else {
        return unavailable_runtime_forecast("database-unavailable", battery_id, &state);
    };

    let session_kind = match kind {
        RuntimeForecastKind::Discharging => storage::BatterySessionKind::Discharging,
        RuntimeForecastKind::Charging => storage::BatterySessionKind::Charging,
    };
    let query = storage::SessionQuery {
        start: OffsetDateTime::UNIX_EPOCH,
        end: OffsetDateTime::now_utc(),
        battery_id: battery_id.clone(),
    };
    let Ok(Some(band)) =
        storage.historical_rate_by_current_percentage(&query, session_kind, current_percentage)
    else {
        return unavailable_runtime_forecast("database-unavailable", battery_id, &state);
    };

    let now = OffsetDateTime::now_utc();
    let recent = battery_id.as_deref().and_then(|id| {
        storage
            .recent_rate_percent_per_hour(
                id,
                now,
                time::Duration::minutes(RUNTIME_FORECAST_LIVE_LOOKBACK_MINUTES),
            )
            .ok()
            .flatten()
    });

    available_runtime_forecast(
        battery_id,
        state,
        kind,
        current_percentage,
        band,
        recent,
        now,
    )
}

/// Builds the "database opened, evidence looked up" branch of
/// `get_runtime_forecast`; split out to keep that command's own length
/// reasonable. `band.stats` being absent (not enough comparable historical
/// sessions yet) is reported plainly rather than guessed at.
#[allow(clippy::needless_pass_by_value)]
fn available_runtime_forecast(
    battery_id: Option<String>,
    state: String,
    kind: RuntimeForecastKind,
    current_percentage: f64,
    band: storage::HistoricalRateBand,
    recent: Option<storage::RecentRateEvidence>,
    now: OffsetDateTime,
) -> RuntimeForecastResponse {
    let Some(rate_stats) = band.stats else {
        return RuntimeForecastResponse {
            schema_version: 1,
            availability: "available",
            unavailable_reason: None,
            generated_at: format_timestamp(now),
            battery_id,
            state: Some(state),
            band_start_percent: Some(band.band_start_percent),
            band_end_percent: Some(band.band_end_percent),
            evidence: "insufficient",
            confidence: "none",
            session_count: 0,
            historical_rate_percent_per_hour: None,
            live_rate_percent_per_hour: recent.map(|value| value.rate_percent_per_hour),
            live_rate_window_minutes: recent.map(|value| value.window_minutes),
            blended_rate_percent_per_hour: None,
            estimated_minutes_remaining: None,
            estimated_at: None,
        };
    };

    let blended_rate =
        storage::blend_rate_percent_per_hour(rate_stats.average_percent_per_hour, recent);
    let target_delta_percent = match kind {
        RuntimeForecastKind::Discharging => current_percentage,
        RuntimeForecastKind::Charging => 100.0 - current_percentage,
    };
    let (estimated_minutes_remaining, estimated_at) = if target_delta_percent <= 0.0 {
        (Some(0.0), format_timestamp(now))
    } else if blended_rate > 0.0 {
        let hours = target_delta_percent / blended_rate;
        let clock_time = now + time::Duration::seconds_f64(hours * 3600.0);
        (Some(hours * 60.0), format_timestamp(clock_time))
    } else {
        (None, None)
    };
    let confidence = if rate_stats.count <= BATTERY_LIFE_LOW_CONFIDENCE_MAX_SESSIONS {
        "low"
    } else if rate_stats.count <= BATTERY_LIFE_MODERATE_CONFIDENCE_MAX_SESSIONS {
        "moderate"
    } else {
        "high"
    };

    RuntimeForecastResponse {
        schema_version: 1,
        availability: "available",
        unavailable_reason: None,
        generated_at: format_timestamp(now),
        battery_id,
        state: Some(state),
        band_start_percent: Some(band.band_start_percent),
        band_end_percent: Some(band.band_end_percent),
        evidence: "sufficient",
        confidence,
        session_count: rate_stats.count,
        historical_rate_percent_per_hour: Some(rate_stats.average_percent_per_hour),
        live_rate_percent_per_hour: recent.map(|value| value.rate_percent_per_hour),
        live_rate_window_minutes: recent.map(|value| value.window_minutes),
        blended_rate_percent_per_hour: Some(blended_rate),
        estimated_minutes_remaining,
        estimated_at,
    }
}

fn unavailable_runtime_forecast(
    reason: &'static str,
    battery_id: Option<String>,
    state: &str,
) -> RuntimeForecastResponse {
    RuntimeForecastResponse {
        schema_version: 1,
        availability: "unavailable",
        unavailable_reason: Some(reason),
        generated_at: None,
        battery_id,
        state: Some(state.to_owned()),
        band_start_percent: None,
        band_end_percent: None,
        evidence: "insufficient",
        confidence: "none",
        session_count: 0,
        historical_rate_percent_per_hour: None,
        live_rate_percent_per_hour: None,
        live_rate_window_minutes: None,
        blended_rate_percent_per_hour: None,
        estimated_minutes_remaining: None,
        estimated_at: None,
    }
}

fn not_applicable_runtime_forecast(
    battery_id: Option<String>,
    state: String,
) -> RuntimeForecastResponse {
    RuntimeForecastResponse {
        schema_version: 1,
        availability: "not-applicable",
        unavailable_reason: None,
        generated_at: format_timestamp(OffsetDateTime::now_utc()),
        battery_id,
        state: Some(state),
        band_start_percent: None,
        band_end_percent: None,
        evidence: "insufficient",
        confidence: "none",
        session_count: 0,
        historical_rate_percent_per_hour: None,
        live_rate_percent_per_hour: None,
        live_rate_window_minutes: None,
        blended_rate_percent_per_hour: None,
        estimated_minutes_remaining: None,
        estimated_at: None,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeForecastResponse {
    schema_version: u8,
    /// `available`, `unavailable`, or `not-applicable` (idle/full/unknown state).
    availability: &'static str,
    unavailable_reason: Option<&'static str>,
    generated_at: Option<String>,
    battery_id: Option<String>,
    state: Option<String>,
    band_start_percent: Option<f64>,
    band_end_percent: Option<f64>,
    evidence: &'static str,
    confidence: &'static str,
    session_count: u64,
    historical_rate_percent_per_hour: Option<f64>,
    live_rate_percent_per_hour: Option<f64>,
    live_rate_window_minutes: Option<f64>,
    blended_rate_percent_per_hour: Option<f64>,
    estimated_minutes_remaining: Option<f64>,
    estimated_at: Option<String>,
}

fn unavailable_today_vs_yesterday(
    reason: &'static str,
    timezone: &str,
) -> TodayVsYesterdayResponse {
    TodayVsYesterdayResponse {
        schema_version: 1,
        availability: "unavailable",
        unavailable_reason: Some(reason),
        generated_at: None,
        timezone: timezone.to_owned(),
        battery_id: None,
        today: DayUsageDto::unavailable(),
        yesterday: DayUsageDto::unavailable(),
    }
}

/// Builds one day's usage DTO. For a specific battery, storage's per-battery
/// evidence and rate math are used directly. For the aggregate "all
/// batteries" scope, coverage/sample-count/net-energy come from the combined
/// read (valid because they never mix distinct batteries' raw series), while
/// percentage change and directional average power are combined explicitly
/// from each contributing battery's own sufficient-evidence summary.
fn day_usage_dto(
    storage: &storage::Storage,
    battery_id: Option<&str>,
    date: NaiveDate,
    day_start: OffsetDateTime,
    day_end: OffsetDateTime,
) -> Option<DayUsageDto> {
    let overall = storage
        .day_usage_summary(&storage::DayUsageQuery {
            battery_id: battery_id.map(str::to_owned),
            start: day_start,
            end: day_end,
        })
        .ok()?;

    let (
        start_percentage,
        end_percentage,
        percentage_change,
        discharge_power,
        charge_power,
        contributing_batteries,
    ) = if battery_id.is_some() {
        (
            overall.start_percentage,
            overall.end_percentage,
            overall.percentage_change,
            overall.average_discharge_power_watts,
            overall.average_charge_power_watts,
            None,
        )
    } else {
        let battery_ids = storage
            .battery_ids_in_range(day_start, day_end)
            .unwrap_or_default();
        let (percentage_change, discharge_power, charge_power, contributing) =
            combine_battery_day_usage(storage, &battery_ids, day_start, day_end);
        (
            None,
            None,
            percentage_change,
            discharge_power,
            charge_power,
            contributing,
        )
    };

    Some(DayUsageDto {
        available: true,
        date: date.format("%Y-%m-%d").to_string(),
        day_start: format_timestamp(day_start),
        day_end: format_timestamp(day_end),
        evidence: match overall.evidence {
            storage::DayEvidence::Sufficient => "sufficient",
            storage::DayEvidence::Insufficient => "insufficient",
        },
        insufficient_reason: overall.insufficiency_reason.map(|reason| match reason {
            storage::DayInsufficiencyReason::NoRecording => "no-recording",
            storage::DayInsufficiencyReason::TooFewSamples => "too-few-samples",
        }),
        sample_count: u64::try_from(overall.sample_count).unwrap_or(u64::MAX),
        elapsed_seconds: overall.elapsed_seconds,
        observed_duration_seconds: overall.observed_duration_seconds,
        coverage_ratio: overall.coverage_ratio,
        start_percentage,
        end_percentage,
        percentage_change,
        energy_change_wh: overall.energy_change_wh,
        average_discharge_power_watts: discharge_power,
        average_charge_power_watts: charge_power,
        contributing_batteries,
    })
}

/// Combines each contributing battery's own sufficient-evidence day summary
/// into one aggregate figure per metric. A battery below the evidence policy
/// is excluded entirely rather than treated as a zero contribution.
fn combine_battery_day_usage(
    storage: &storage::Storage,
    battery_ids: &[String],
    day_start: OffsetDateTime,
    day_end: OffsetDateTime,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<u32>) {
    let mut weighted_percentage_sum = 0.0;
    let mut weight_sum = 0.0;
    let mut percentage_fully_known = true;
    let mut discharge_sum: Option<f64> = None;
    let mut charge_sum: Option<f64> = None;
    let mut contributing = 0_u32;
    for battery_id in battery_ids {
        let Ok(summary) = storage.day_usage_summary(&storage::DayUsageQuery {
            battery_id: Some(battery_id.clone()),
            start: day_start,
            end: day_end,
        }) else {
            continue;
        };
        if summary.evidence != storage::DayEvidence::Sufficient {
            continue;
        }
        contributing += 1;
        match (
            summary.percentage_change,
            summary.representative_full_energy_wh,
        ) {
            (Some(change), Some(weight)) if weight > 0.0 => {
                weighted_percentage_sum += change * weight;
                weight_sum += weight;
            }
            _ => percentage_fully_known = false,
        }
        if let Some(power) = summary.average_discharge_power_watts {
            discharge_sum = Some(discharge_sum.unwrap_or(0.0) + power);
        }
        if let Some(power) = summary.average_charge_power_watts {
            charge_sum = Some(charge_sum.unwrap_or(0.0) + power);
        }
    }
    let percentage_change = (percentage_fully_known && weight_sum > 0.0)
        .then_some(weighted_percentage_sum / weight_sum);
    (
        percentage_change,
        discharge_sum,
        charge_sum,
        (contributing > 0).then_some(contributing),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TodayVsYesterdayResponse {
    schema_version: u8,
    availability: &'static str,
    unavailable_reason: Option<&'static str>,
    generated_at: Option<String>,
    timezone: String,
    /// The requested scope: `None` means the aggregate "all batteries" view.
    battery_id: Option<String>,
    today: DayUsageDto,
    yesterday: DayUsageDto,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DayUsageDto {
    available: bool,
    date: String,
    day_start: Option<String>,
    day_end: Option<String>,
    evidence: &'static str,
    insufficient_reason: Option<&'static str>,
    sample_count: u64,
    elapsed_seconds: f64,
    observed_duration_seconds: Option<f64>,
    coverage_ratio: Option<f64>,
    start_percentage: Option<f64>,
    end_percentage: Option<f64>,
    percentage_change: Option<f64>,
    energy_change_wh: Option<f64>,
    average_discharge_power_watts: Option<f64>,
    average_charge_power_watts: Option<f64>,
    /// Set only for the aggregate scope: how many physical batteries cleared
    /// the evidence policy and contributed to the combined figures.
    contributing_batteries: Option<u32>,
}
impl DayUsageDto {
    fn unavailable() -> Self {
        Self {
            available: false,
            date: String::new(),
            day_start: None,
            day_end: None,
            evidence: "insufficient",
            insufficient_reason: Some("no-recording"),
            sample_count: 0,
            elapsed_seconds: 0.0,
            observed_duration_seconds: None,
            coverage_ratio: None,
            start_percentage: None,
            end_percentage: None,
            percentage_change: None,
            energy_change_wh: None,
            average_discharge_power_watts: None,
            average_charge_power_watts: None,
            contributing_batteries: None,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionHistoryResponse {
    schema_version: u8,
    availability: &'static str,
    unavailable_reason: Option<&'static str>,
    generated_at: Option<String>,
    timezone: String,
    sessions: Vec<SessionDto>,
    daily: Vec<CalendarDto>,
    weekly: Vec<CalendarDto>,
    monthly: Vec<CalendarDto>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionDto {
    id: String,
    battery_id: Option<String>,
    state: &'static str,
    started_at: Option<String>,
    ended_at: Option<String>,
    duration_seconds: Option<f64>,
    start_percentage: Option<f64>,
    end_percentage: Option<f64>,
    start_energy_wh: Option<f64>,
    end_energy_wh: Option<f64>,
    transferred_energy_wh: Option<f64>,
    average_power_watts: Option<f64>,
    peak_power_watts: Option<f64>,
    completeness: &'static str,
    boundary_reason: &'static str,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CalendarDto {
    period: &'static str,
    bucket: String,
    timezone: String,
    battery_id: Option<String>,
    observed_energy_used_wh: Option<f64>,
    observed_energy_charged_wh: Option<f64>,
    minimum_percentage: Option<f64>,
    maximum_percentage: Option<f64>,
    representative_full_energy_wh: Option<f64>,
    coverage_seconds: Option<f64>,
    coverage_ratio: Option<f64>,
    observed_samples: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionRebuildResponse {
    schema_version: u8,
    availability: &'static str,
    unavailable_reason: Option<&'static str>,
    rebuilt_at: Option<String>,
    sessions_rebuilt: Option<u64>,
}
impl SessionRebuildResponse {
    fn unavailable(reason: &'static str) -> Self {
        Self {
            schema_version: 1,
            availability: "unavailable",
            unavailable_reason: Some(reason),
            rebuilt_at: None,
            sessions_rebuilt: None,
        }
    }
}

fn unavailable_session_history(reason: &'static str, timezone: &str) -> SessionHistoryResponse {
    SessionHistoryResponse {
        schema_version: 1,
        availability: "unavailable",
        unavailable_reason: Some(reason),
        generated_at: None,
        timezone: timezone.to_owned(),
        sessions: Vec::new(),
        daily: Vec::new(),
        weekly: Vec::new(),
        monthly: Vec::new(),
    }
}
fn session_kind(kind: storage::BatterySessionKind) -> &'static str {
    match kind {
        storage::BatterySessionKind::Charging => "charging",
        storage::BatterySessionKind::Discharging => "discharging",
        storage::BatterySessionKind::Full => "full",
        storage::BatterySessionKind::Unknown => "unknown",
    }
}
fn normalize_session_states(values: &[String]) -> Option<BTreeSet<String>> {
    let set = values.iter().cloned().collect::<BTreeSet<_>>();
    (!set.is_empty()
        && set.iter().all(|value| {
            matches!(
                value.as_str(),
                "charging" | "discharging" | "full" | "unknown"
            )
        }))
    .then_some(set)
}
fn map_session(session: storage::BatterySession, index: usize) -> SessionDto {
    let energy = match (session.start_energy_wh, session.end_energy_wh) {
        (Some(start), Some(end)) => Some(end - start),
        _ => None,
    };
    SessionDto {
        id: format!("{}:{}:{}", session.battery_id, session.started_at, index),
        battery_id: Some(session.battery_id),
        state: session_kind(session.kind),
        started_at: Some(session.started_at),
        ended_at: Some(session.ended_at),
        duration_seconds: session.observed_duration_seconds,
        start_percentage: session.start_percentage,
        end_percentage: session.end_percentage,
        start_energy_wh: session.start_energy_wh,
        end_energy_wh: session.end_energy_wh,
        transferred_energy_wh: energy,
        average_power_watts: session.average_power_watts,
        peak_power_watts: None,
        completeness: if session.complete {
            "complete"
        } else {
            "incomplete"
        },
        boundary_reason: match session.interrupt_reason {
            storage::SessionInterruptReason::StateChanged => "state-change",
            storage::SessionInterruptReason::BootChanged => "rebooted",
            storage::SessionInterruptReason::SampleGap => "sampling-gap",
            storage::SessionInterruptReason::DataEnded => "end-of-data",
        },
    }
}

fn session_date_range(
    timezone: Tz,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<(OffsetDateTime, OffsetDateTime), ()> {
    let today = Utc::now().with_timezone(&timezone).date_naive();
    let start = start
        .map(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d"))
        .transpose()
        .map_err(|_| ())?
        .unwrap_or(NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid epoch date"));
    let end = end
        .map(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d"))
        .transpose()
        .map_err(|_| ())?
        .unwrap_or(today);
    if end < start {
        return Err(());
    }
    let start = local_day_start(timezone, start)?;
    let end = local_day_start(timezone, end.succ_opt().ok_or(())?)?;
    Ok((
        chrono_utc_to_offset_datetime(start)?,
        chrono_utc_to_offset_datetime(end)?,
    ))
}
fn local_day_start(timezone: Tz, day: NaiveDate) -> Result<chrono::DateTime<Utc>, ()> {
    (0..=4)
        .find_map(|hour| {
            timezone
                .with_ymd_and_hms(day.year(), day.month(), day.day(), hour, 0, 0)
                .earliest()
        })
        .map(|value| value.with_timezone(&Utc))
        .ok_or(())
}
fn chrono_utc_to_offset_datetime(value: chrono::DateTime<Utc>) -> Result<OffsetDateTime, ()> {
    OffsetDateTime::from_unix_timestamp(value.timestamp())
        .map_err(|_| ())
        .map(|value| value + time::Duration::nanoseconds(i64::from(value.nanosecond())))
}
/// Resolves the inclusive local-calendar-day boundaries for one date as UTC
/// instants, reusing the same IANA/DST-aware midnight resolution as session
/// history bucketing (`local_day_start`) so today/yesterday grouping never
/// diverges from the rest of the app's calendar semantics.
fn local_day_bounds(timezone: Tz, day: NaiveDate) -> Result<(OffsetDateTime, OffsetDateTime), ()> {
    let start = local_day_start(timezone, day)?;
    let end = local_day_start(timezone, day.succ_opt().ok_or(())?)?;
    Ok((
        chrono_utc_to_offset_datetime(start)?,
        chrono_utc_to_offset_datetime(end)?,
    ))
}
fn calendar_summaries(
    sessions: &[storage::BatterySession],
    timezone: Tz,
    period: &'static str,
) -> Vec<CalendarDto> {
    let mut buckets = BTreeMap::<(String, String), CalendarDto>::new();
    for session in sessions {
        let Ok(start) = chrono::DateTime::parse_from_rfc3339(&session.started_at) else {
            continue;
        };
        let local = start.with_timezone(&timezone);
        let bucket = match period {
            "daily" => local.format("%F").to_string(),
            "weekly" => {
                let week = local.iso_week();
                format!("{:04}-W{:02}", week.year(), week.week())
            }
            _ => local.format("%Y-%m").to_string(),
        };
        let key = (bucket.clone(), session.battery_id.clone());
        let entry = buckets.entry(key).or_insert_with(|| CalendarDto {
            period,
            bucket,
            timezone: timezone.name().to_owned(),
            battery_id: Some(session.battery_id.clone()),
            observed_energy_used_wh: None,
            observed_energy_charged_wh: None,
            minimum_percentage: None,
            maximum_percentage: None,
            representative_full_energy_wh: None,
            coverage_seconds: Some(0.0),
            coverage_ratio: None,
            observed_samples: 0,
        });
        entry.observed_samples += session.sample_count;
        // One incomplete or still-open session without a known duration must
        // not erase the recorded time already measured from every other
        // session in this bucket -- that used to zero out the whole
        // period's "Recorded time" the moment a single boundary session
        // (for example one interrupted immediately by a sampling gap) had
        // no `observed_duration_seconds`, even on a day with hours of real,
        // measured session coverage. Only a session that does carry a
        // duration contributes to the running total; one that does not
        // simply leaves the accumulated total unchanged, mirroring how
        // `minimum_percentage`/`maximum_percentage` above tolerate
        // individual sessions with missing data.
        if let Some(value) = session.observed_duration_seconds {
            entry.coverage_seconds = Some(entry.coverage_seconds.unwrap_or(0.0) + value);
        }
        entry.minimum_percentage = match (
            entry.minimum_percentage,
            session.start_percentage,
            session.end_percentage,
        ) {
            (Some(current), Some(start), Some(end)) => Some(current.min(start).min(end)),
            (None, Some(start), Some(end)) => Some(start.min(end)),
            (current, _, _) => current,
        };
        entry.maximum_percentage = match (
            entry.maximum_percentage,
            session.start_percentage,
            session.end_percentage,
        ) {
            (Some(current), Some(start), Some(end)) => Some(current.max(start).max(end)),
            (None, Some(start), Some(end)) => Some(start.max(end)),
            (current, _, _) => current,
        };
        if let (Some(start), Some(end)) = (session.start_energy_wh, session.end_energy_wh) {
            let change = end - start;
            if change < 0.0 {
                entry.observed_energy_used_wh =
                    Some(entry.observed_energy_used_wh.unwrap_or(0.0) - change);
            } else {
                entry.observed_energy_charged_wh =
                    Some(entry.observed_energy_charged_wh.unwrap_or(0.0) + change);
            }
        }
    }
    buckets.into_values().collect()
}

/// A user-initiated export request.  The destination is intentionally a
/// required caller value: the command never invents an application path or
/// silently writes into the database directory.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportRequest {
    data_type: String,
    format: String,
    destination: String,
    #[serde(default)]
    battery_id: Option<String>,
    #[serde(default)]
    start_date: Option<String>,
    #[serde(default)]
    end_date: Option<String>,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    summary_period: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExportDataType {
    RawSamples,
    Sessions,
    Summaries,
}

impl ExportDataType {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "raw-samples" | "raw_samples" | "rawSamples" => Some(Self::RawSamples),
            "sessions" => Some(Self::Sessions),
            "summaries" | "calendar-summaries" | "calendar_summaries" => Some(Self::Summaries),
            _ => None,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::RawSamples => "raw-samples",
            Self::Sessions => "sessions",
            Self::Summaries => "summaries",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportResponse {
    schema_version: u8,
    availability: &'static str,
    unavailable_reason: Option<&'static str>,
    data_type: String,
    format: String,
    destination: Option<String>,
    record_count: usize,
    bytes_written: Option<u64>,
    error: Option<String>,
}

/// Exports immutable local history only after the caller supplies an explicit
/// destination selected by the user.  The writer refuses to replace an
/// existing file, including when a competing process creates it during the
/// export.
#[tauri::command]
#[allow(clippy::too_many_lines)]
fn export_battery_history(request: ExportRequest) -> ExportResponse {
    let requested_data_type = request.data_type.clone();
    let requested_format = request.format.to_ascii_lowercase();
    let Some(data_type) = ExportDataType::parse(&requested_data_type) else {
        return failed_export(
            requested_data_type,
            requested_format,
            Some(request.destination),
            "invalid-request",
            "data_type must be raw-samples, sessions, or summaries",
        );
    };
    let format = match requested_format.as_str() {
        "csv" => export::ExportFormat::Csv,
        "json" => export::ExportFormat::Json,
        _ => {
            return failed_export(
                data_type.as_str().to_owned(),
                requested_format,
                Some(request.destination),
                "invalid-request",
                "format must be csv or json",
            );
        }
    };
    if request.destination.trim().is_empty() {
        return failed_export(
            data_type.as_str().to_owned(),
            format_name(format).to_owned(),
            None,
            "invalid-request",
            "an explicit user-selected destination is required",
        );
    }
    let destination = PathBuf::from(&request.destination);
    if !destination.is_absolute() {
        return failed_export(
            data_type.as_str().to_owned(),
            format_name(format).to_owned(),
            Some(request.destination),
            "invalid-request",
            "the export destination must be an absolute user-selected path",
        );
    }
    if request
        .battery_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return failed_export(
            data_type.as_str().to_owned(),
            format_name(format).to_owned(),
            Some(request.destination),
            "invalid-request",
            "battery_id must not be empty when supplied",
        );
    }

    let timezone_name = request.timezone.as_deref().unwrap_or("UTC");
    let Ok(timezone) = timezone_name.parse::<Tz>() else {
        return failed_export(
            data_type.as_str().to_owned(),
            format_name(format).to_owned(),
            Some(request.destination),
            "invalid-request",
            "timezone must be a valid IANA timezone",
        );
    };
    let Ok((start, end)) = export_date_range(
        timezone,
        request.start_date.as_deref(),
        request.end_date.as_deref(),
    ) else {
        return failed_export(
            data_type.as_str().to_owned(),
            format_name(format).to_owned(),
            Some(request.destination),
            "invalid-request",
            "start_date and end_date must be ordered YYYY-MM-DD values",
        );
    };

    let metadata = export::ExportMetadata {
        generated_at: format_timestamp(OffsetDateTime::now_utc())
            .expect("UTC timestamps are representable as RFC 3339"),
        timezone: timezone.name().to_owned(),
    };
    let records = match load_export_records(
        data_type,
        request.battery_id,
        start,
        end,
        timezone,
        request.summary_period.as_deref(),
    ) {
        Ok(records) => records,
        Err(ExportLoadError::Unavailable(reason)) => {
            return failed_export(
                data_type.as_str().to_owned(),
                format_name(format).to_owned(),
                Some(request.destination),
                reason,
                export_unavailable_message(reason),
            );
        }
        Err(ExportLoadError::Invalid(message)) => {
            return failed_export(
                data_type.as_str().to_owned(),
                format_name(format).to_owned(),
                Some(request.destination),
                "invalid-request",
                message,
            );
        }
        Err(ExportLoadError::Database) => {
            return failed_export(
                data_type.as_str().to_owned(),
                format_name(format).to_owned(),
                Some(request.destination),
                "database-unavailable",
                "the local history database could not be read",
            );
        }
    };
    let record_count = export_record_count(&records);
    if record_count == 0 {
        return failed_export(
            data_type.as_str().to_owned(),
            format_name(format).to_owned(),
            Some(request.destination),
            "no-recorded-samples",
            "no recorded rows match the export request",
        );
    }

    let document = export::ExportDocument { metadata, records };
    match export::write_export(&destination, &document, format) {
        Ok(()) => ExportResponse {
            schema_version: 1,
            availability: "available",
            unavailable_reason: None,
            data_type: data_type.as_str().to_owned(),
            format: format_name(format).to_owned(),
            destination: Some(request.destination),
            record_count,
            bytes_written: std::fs::metadata(&destination)
                .ok()
                .map(|metadata| metadata.len()),
            error: None,
        },
        Err(error) => {
            let reason = match error {
                export::ExportError::DestinationExists(_) => "destination-exists",
                export::ExportError::InvalidPath(_) => "invalid-destination",
                export::ExportError::Io(_) => "destination-write-failed",
            };
            failed_export(
                data_type.as_str().to_owned(),
                format_name(format).to_owned(),
                Some(request.destination),
                reason,
                &error.to_string(),
            )
        }
    }
}

fn format_name(format: export::ExportFormat) -> &'static str {
    match format {
        export::ExportFormat::Csv => "csv",
        export::ExportFormat::Json => "json",
    }
}

fn failed_export(
    data_type: String,
    format: String,
    destination: Option<String>,
    reason: &'static str,
    message: &str,
) -> ExportResponse {
    ExportResponse {
        schema_version: 1,
        availability: "unavailable",
        unavailable_reason: Some(reason),
        data_type,
        format,
        destination,
        record_count: 0,
        bytes_written: None,
        error: Some(message.to_owned()),
    }
}

fn export_unavailable_message(reason: &str) -> &'static str {
    match reason {
        "no-recorded-samples" => "no recorded rows match the export request",
        "recorder-disabled" => "recording is disabled and no history is available",
        "unsupported" => "local recording is unsupported on this system",
        _ => "no local history is available",
    }
}

fn export_date_range(
    timezone: Tz,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<(OffsetDateTime, OffsetDateTime), ()> {
    if start.is_none() && end.is_none() {
        return Ok((OffsetDateTime::UNIX_EPOCH, OffsetDateTime::now_utc()));
    }
    session_date_range(timezone, start, end)
}

enum ExportLoadError {
    Unavailable(&'static str),
    Invalid(&'static str),
    Database,
}

fn load_export_records(
    data_type: ExportDataType,
    battery_id: Option<String>,
    start: OffsetDateTime,
    end: OffsetDateTime,
    timezone: Tz,
    summary_period: Option<&str>,
) -> Result<export::ExportRecords, ExportLoadError> {
    match data_type {
        ExportDataType::RawSamples => {
            let query = storage::HistoryQuery {
                start,
                end,
                battery_id,
                max_points: usize::MAX,
            };
            let history = storage::history_if_exists(&query)
                .map_err(|_| ExportLoadError::Database)?
                .ok_or_else(|| {
                    ExportLoadError::Unavailable(recorder_unavailable_reason(recorder_state()))
                })?;
            let samples = history_samples(&history);
            if samples.is_empty() {
                return Err(ExportLoadError::Unavailable("no-recorded-samples"));
            }
            Ok(export::ExportRecords::RawSamples(samples))
        }
        ExportDataType::Sessions | ExportDataType::Summaries => {
            let Some(path) =
                storage::existing_database_path().map_err(|_| ExportLoadError::Database)?
            else {
                return Err(ExportLoadError::Unavailable(recorder_unavailable_reason(
                    recorder_state(),
                )));
            };
            let database =
                storage::Storage::open_at(path).map_err(|_| ExportLoadError::Database)?;
            let query = storage::SessionQuery {
                start,
                end,
                battery_id,
            };
            let sessions = database
                .sessions(&query)
                .map_err(|_| ExportLoadError::Database)?;
            if sessions.is_empty() {
                return Err(ExportLoadError::Unavailable("no-recorded-samples"));
            }
            if data_type == ExportDataType::Sessions {
                return Ok(export::ExportRecords::Sessions(sessions));
            }
            let period = summary_period.unwrap_or("daily");
            if !matches!(period, "daily" | "weekly" | "monthly") {
                return Err(ExportLoadError::Invalid(
                    "summary_period must be daily, weekly, or monthly",
                ));
            }
            let summaries = export_session_summaries(&sessions, timezone, period);
            if summaries.is_empty() {
                return Err(ExportLoadError::Unavailable("no-recorded-samples"));
            }
            Ok(export::ExportRecords::Summaries(summaries))
        }
    }
}

fn export_record_count(records: &export::ExportRecords) -> usize {
    match records {
        export::ExportRecords::RawSamples(records) => records.len(),
        export::ExportRecords::Sessions(records) => records.len(),
        export::ExportRecords::Summaries(records) => records.len(),
    }
}

fn export_session_summaries(
    sessions: &[storage::BatterySession],
    timezone: Tz,
    period: &str,
) -> Vec<storage::SessionAggregation> {
    let mut buckets = BTreeMap::<(String, String), (u64, u64, Option<f64>)>::new();
    for session in sessions {
        let Ok(start) = chrono::DateTime::parse_from_rfc3339(&session.started_at) else {
            continue;
        };
        let Ok(end) = chrono::DateTime::parse_from_rfc3339(&session.ended_at) else {
            continue;
        };
        let local_start = start.with_timezone(&timezone);
        let local_end = end.with_timezone(&timezone);
        let bucket = export_calendar_bucket(local_start, period);
        let crosses_boundary = bucket != export_calendar_bucket(local_end, period);
        let entry = buckets
            .entry((bucket, session.battery_id.clone()))
            .or_insert((0, 0, Some(0.0)));
        entry.0 += 1;
        entry.1 += u64::from(session.complete);
        entry.2 = match (entry.2, session.observed_duration_seconds, crosses_boundary) {
            (Some(total), Some(duration), false) => Some(total + duration),
            _ => None,
        };
    }
    buckets
        .into_iter()
        .map(
            |(
                (bucket, battery_id),
                (session_count, complete_session_count, observed_duration_seconds),
            )| {
                storage::SessionAggregation {
                    bucket,
                    battery_id,
                    session_count,
                    complete_session_count,
                    observed_duration_seconds,
                }
            },
        )
        .collect()
}

fn export_calendar_bucket(timestamp: chrono::DateTime<Tz>, period: &str) -> String {
    match period {
        "daily" => timestamp.format("%F").to_string(),
        "weekly" => {
            let week = timestamp.iso_week();
            format!("{:04}-W{:02}", week.year(), week.week())
        }
        _ => timestamp.format("%Y-%m").to_string(),
    }
}

#[derive(Default)]
struct MappedHistory {
    points: Vec<RecentHistoryPoint>,
    gaps: Vec<RecentHistoryGap>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecentHistoryMetric {
    value: Option<f64>,
    source: &'static str,
    availability: &'static str,
    observed_at: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecentHistoryMetrics {
    percentage: RecentHistoryMetric,
    energy_now_wh: RecentHistoryMetric,
    power_watts: RecentHistoryMetric,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecentHistoryPoint {
    battery_id: String,
    recorded_at: String,
    kind: &'static str,
    state: &'static str,
    freshness: &'static str,
    metrics: RecentHistoryMetrics,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecentHistoryGap {
    starts_at: String,
    ends_at: Option<String>,
    reason: &'static str,
    detail: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NumericHistorySummary {
    minimum: Option<f64>,
    maximum: Option<f64>,
    average: Option<f64>,
    /// When the recorded minimum value was actually observed. Distinct from
    /// `observed_at`, which is the last persisted sample in the window: the
    /// user reasonably expects to know *when* an extremum happened, not only
    /// what the extremum was.
    minimum_at: Option<String>,
    /// When the recorded maximum value was actually observed.
    maximum_at: Option<String>,
    observed_samples: usize,
    source: &'static str,
    availability: &'static str,
    observed_at: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservedEnergySummary {
    first: Option<f64>,
    last: Option<f64>,
    change: Option<f64>,
    observed_samples: usize,
    source: &'static str,
    availability: &'static str,
    observed_at: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecentBatteryHistorySummary {
    percentage: NumericHistorySummary,
    power_watts: NumericHistorySummary,
    energy_now_wh: NumericHistorySummary,
    observed_energy_wh: ObservedEnergySummary,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecentBatteryHistoryResponse {
    schema_version: u8,
    availability: &'static str,
    unavailable_reason: Option<&'static str>,
    source: &'static str,
    freshness: &'static str,
    battery_id: Option<String>,
    range_hours: u16,
    collected_at: Option<String>,
    points: Vec<RecentHistoryPoint>,
    gaps: Vec<RecentHistoryGap>,
    summary: RecentBatteryHistorySummary,
}

/// Fixed windows supported by the recent-history dashboard chart, from a
/// short 2-hour zoom up to a 30-day (720-hour) overview. `3d`/`7d`/`30d` are
/// expressed in hours (72/168/720) so the backend keeps a single unit.
fn supported_history_range(range_hours: u16) -> Option<u16> {
    matches!(range_hours, 2 | 6 | 12 | 24 | 72 | 168 | 720).then_some(range_hours)
}

fn unavailable_history(
    battery_id: Option<String>,
    range_hours: u16,
    reason: &'static str,
) -> RecentBatteryHistoryResponse {
    RecentBatteryHistoryResponse {
        schema_version: 1,
        availability: "unavailable",
        unavailable_reason: Some(reason),
        source: "unavailable",
        freshness: "unknown",
        battery_id,
        range_hours,
        collected_at: None,
        points: Vec::new(),
        gaps: Vec::new(),
        summary: empty_history_summary(),
    }
}

fn map_persisted_history(history: &storage::HistoryResponse) -> MappedHistory {
    let samples = history
        .timeline
        .iter()
        .filter_map(|item| match item {
            storage::HistoryTimelineItem::Sample(sample) => Some(sample.as_ref()),
            storage::HistoryTimelineItem::Gap(_) => None,
        })
        .collect::<Vec<_>>();
    let mut mapped = if history.battery_id.is_some() {
        MappedHistory {
            points: samples.into_iter().map(persisted_history_point).collect(),
            gaps: Vec::new(),
        }
    } else {
        aggregate_persisted_history(&samples)
    };

    mapped
        .gaps
        .extend(history.timeline.iter().filter_map(|item| match item {
            storage::HistoryTimelineItem::Gap(gap) => Some(RecentHistoryGap {
                starts_at: gap.from.clone(),
                ends_at: Some(gap.to.clone()),
                reason: match gap.reason {
                    storage::HistoryGapReason::BootChanged => "rebooted",
                    storage::HistoryGapReason::SampleIntervalExceeded => "missing-samples",
                },
                detail: None,
            }),
            storage::HistoryTimelineItem::Sample(_) => None,
        }));
    mapped.gaps.sort_by(|left, right| {
        left.starts_at
            .cmp(&right.starts_at)
            .then(left.ends_at.cmp(&right.ends_at))
    });
    mapped.gaps.dedup_by(|left, right| {
        left.starts_at == right.starts_at
            && left.ends_at == right.ends_at
            && left.reason == right.reason
    });
    mapped
}

fn persisted_history_point(sample: &storage::HistorySample) -> RecentHistoryPoint {
    RecentHistoryPoint {
        battery_id: sample.battery_id.clone(),
        recorded_at: sample.recorded_at.clone(),
        kind: "persisted",
        state: storage_state(sample.state),
        freshness: "fresh",
        metrics: RecentHistoryMetrics {
            percentage: persisted_metric(sample.metrics.percentage, &sample.recorded_at),
            energy_now_wh: persisted_metric(sample.metrics.energy_now_wh, &sample.recorded_at),
            power_watts: persisted_metric(sample.metrics.power_watts, &sample.recorded_at),
        },
    }
}

fn aggregate_persisted_history(samples: &[&storage::HistorySample]) -> MappedHistory {
    // Some discovered power-supply batteries never yield a usable percentage
    // anywhere in the window -- for example a paired wireless mouse or
    // keyboard exposes a `hidpp_battery_*` power supply that UPower reports
    // with every field `unavailable`. Requiring the aggregate view to see
    // such a battery at every instant before treating that instant as
    // "complete" would permanently block the combined percentage/energy
    // series and manufacture a "missing-samples" gap at nearly every
    // timestamp, turning the chart into a wall of gap markers instead of a
    // battery-level line. Completeness and gap detection only consider
    // batteries that have contributed at least one real percentage reading
    // somewhere in this window; a battery that never contributes usable data
    // has nothing for the aggregate to lose by being excluded.
    let contributing_ids = samples
        .iter()
        .filter(|sample| sample.metrics.percentage.value.is_some())
        .map(|sample| sample.battery_id.as_str())
        .collect::<BTreeSet<_>>();
    let samples = samples
        .iter()
        .copied()
        .filter(|sample| contributing_ids.contains(sample.battery_id.as_str()))
        .collect::<Vec<_>>();
    let samples = samples.as_slice();

    let expected_ids = samples
        .iter()
        .map(|sample| sample.battery_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut groups = BTreeMap::<&str, Vec<&storage::HistorySample>>::new();
    for sample in samples {
        groups.entry(&sample.recorded_at).or_default().push(*sample);
    }
    let mut mapped = MappedHistory::default();
    for (timestamp, group) in groups {
        let present_ids = group
            .iter()
            .map(|sample| sample.battery_id.as_str())
            .collect::<BTreeSet<_>>();
        if present_ids != expected_ids {
            mapped.gaps.push(RecentHistoryGap {
                starts_at: timestamp.to_owned(),
                ends_at: Some(timestamp.to_owned()),
                reason: "missing-samples",
                detail: Some(
                    "Not every discovered battery was sampled at this instant.".to_owned(),
                ),
            });
        }
        mapped.points.push(aggregate_persisted_point(
            timestamp,
            &group,
            present_ids == expected_ids,
        ));
    }
    mapped
}

fn aggregate_persisted_point(
    timestamp: &str,
    samples: &[&storage::HistorySample],
    complete: bool,
) -> RecentHistoryPoint {
    let state = samples.first().map_or("unknown", |first| {
        let first_state = storage_state(first.state);
        if samples
            .iter()
            .all(|sample| storage_state(sample.state) == first_state)
        {
            first_state
        } else {
            "unknown"
        }
    });
    RecentHistoryPoint {
        battery_id: "all-batteries".to_owned(),
        recorded_at: timestamp.to_owned(),
        kind: "persisted",
        state,
        freshness: "fresh",
        metrics: RecentHistoryMetrics {
            percentage: aggregate_percentage(samples, timestamp, complete),
            energy_now_wh: aggregate_metric(samples, timestamp, complete, |sample| {
                sample.metrics.energy_now_wh
            }),
            power_watts: aggregate_metric(samples, timestamp, complete, |sample| {
                sample.metrics.power_watts
            }),
        },
    }
}

fn persisted_metric(metric: storage::HistoryMetric, observed_at: &str) -> RecentHistoryMetric {
    let available = metric.value.is_some()
        && matches!(metric.availability, storage::HistoryAvailability::Available);
    RecentHistoryMetric {
        value: available.then_some(metric.value).flatten(),
        source: if available {
            storage_metric_source(metric.source)
        } else {
            "unavailable"
        },
        availability: if available {
            "available"
        } else {
            "unavailable"
        },
        observed_at: available.then(|| observed_at.to_owned()),
    }
}

fn aggregate_metric(
    samples: &[&storage::HistorySample],
    observed_at: &str,
    complete: bool,
    select: impl Fn(&storage::HistorySample) -> storage::HistoryMetric,
) -> RecentHistoryMetric {
    let values = samples
        .iter()
        .map(|sample| select(sample).value)
        .collect::<Option<Vec<_>>>();
    let value = complete
        .then_some(values)
        .flatten()
        .map(|values| values.iter().sum());
    let (source, availability) = aggregate_metric_status(value);
    RecentHistoryMetric {
        value,
        source,
        availability,
        observed_at: value.is_some().then(|| observed_at.to_owned()),
    }
}

fn aggregate_percentage(
    samples: &[&storage::HistorySample],
    observed_at: &str,
    complete: bool,
) -> RecentHistoryMetric {
    let parts = samples
        .iter()
        .map(|sample| {
            Some((
                sample.metrics.percentage.value?,
                sample.metrics.energy_full_wh.value?,
            ))
        })
        .collect::<Option<Vec<_>>>();
    let value = complete.then_some(parts).flatten().and_then(|parts| {
        let capacity = parts.iter().map(|(_, capacity)| capacity).sum::<f64>();
        (capacity > 0.0).then(|| {
            parts
                .iter()
                .map(|(percentage, capacity)| percentage * capacity)
                .sum::<f64>()
                / capacity
        })
    });
    let (source, availability) = aggregate_metric_status(value);
    RecentHistoryMetric {
        value,
        source,
        availability,
        observed_at: value.is_some().then(|| observed_at.to_owned()),
    }
}

fn transient_live_point(
    dashboard: &battery::BatteryDashboardResponse,
    battery_id: Option<&str>,
) -> Option<RecentHistoryPoint> {
    let timestamp = dashboard.collected_at.clone()?;
    if let Some(battery_id) = battery_id {
        return dashboard
            .batteries
            .iter()
            .find(|battery| battery.id == battery_id)
            .map(|battery| live_history_point(battery, &timestamp));
    }
    aggregate_live_history_point(&dashboard.batteries, &timestamp)
}

fn live_history_point(battery: &battery::BatteryResponse, timestamp: &str) -> RecentHistoryPoint {
    RecentHistoryPoint {
        battery_id: battery.id.clone(),
        recorded_at: timestamp.to_owned(),
        kind: "transient",
        state: battery.state,
        freshness: "fresh",
        metrics: RecentHistoryMetrics {
            percentage: live_metric(&battery.metrics.percentage, timestamp),
            energy_now_wh: live_metric(&battery.metrics.energy_now_wh, timestamp),
            power_watts: live_metric(&battery.metrics.power_watts, timestamp),
        },
    }
}

fn aggregate_live_history_point(
    batteries: &[battery::BatteryResponse],
    timestamp: &str,
) -> Option<RecentHistoryPoint> {
    (!batteries.is_empty()).then(|| {
        let state = batteries.first().map_or("unknown", |first| {
            if batteries.iter().all(|battery| battery.state == first.state) {
                first.state
            } else {
                "unknown"
            }
        });
        RecentHistoryPoint {
            battery_id: "all-batteries".to_owned(),
            recorded_at: timestamp.to_owned(),
            kind: "transient",
            state,
            freshness: "fresh",
            metrics: RecentHistoryMetrics {
                percentage: aggregate_live_percentage(batteries, timestamp),
                energy_now_wh: aggregate_live_metric(batteries, timestamp, |battery| {
                    &battery.metrics.energy_now_wh
                }),
                power_watts: aggregate_live_metric(batteries, timestamp, |battery| {
                    &battery.metrics.power_watts
                }),
            },
        }
    })
}

fn live_metric(metric: &battery::MetricResponse, timestamp: &str) -> RecentHistoryMetric {
    let available = metric.value.is_some() && metric.availability == "available";
    RecentHistoryMetric {
        value: available.then_some(metric.value).flatten(),
        source: if available {
            metric.source
        } else {
            "unavailable"
        },
        availability: if available {
            "available"
        } else {
            "unavailable"
        },
        observed_at: available.then(|| timestamp.to_owned()),
    }
}

fn aggregate_live_metric(
    batteries: &[battery::BatteryResponse],
    timestamp: &str,
    select: impl Fn(&battery::BatteryResponse) -> &battery::MetricResponse,
) -> RecentHistoryMetric {
    let values = batteries
        .iter()
        .map(|battery| {
            let metric = select(battery);
            (metric.availability == "available")
                .then_some(metric.value)
                .flatten()
        })
        .collect::<Option<Vec<_>>>();
    let value = values.map(|values| values.iter().sum());
    let (source, availability) = aggregate_metric_status(value);
    RecentHistoryMetric {
        value,
        source,
        availability,
        observed_at: value.is_some().then(|| timestamp.to_owned()),
    }
}

fn aggregate_live_percentage(
    batteries: &[battery::BatteryResponse],
    timestamp: &str,
) -> RecentHistoryMetric {
    let parts = batteries
        .iter()
        .map(|battery| {
            let percentage = &battery.metrics.percentage;
            let capacity = &battery.metrics.energy_full_wh;
            (percentage.availability == "available" && capacity.availability == "available")
                .then_some((percentage.value?, capacity.value?))
        })
        .collect::<Option<Vec<_>>>();
    let value = parts.and_then(|parts| {
        let capacity = parts.iter().map(|(_, capacity)| capacity).sum::<f64>();
        (capacity > 0.0).then(|| {
            parts
                .iter()
                .map(|(percentage, capacity)| percentage * capacity)
                .sum::<f64>()
                / capacity
        })
    });
    let (source, availability) = aggregate_metric_status(value);
    RecentHistoryMetric {
        value,
        source,
        availability,
        observed_at: value.is_some().then(|| timestamp.to_owned()),
    }
}

fn history_summary_from_points(
    points: &[RecentHistoryPoint],
    gaps: &[RecentHistoryGap],
) -> RecentBatteryHistorySummary {
    let persisted = points
        .iter()
        .filter(|point| point.kind == "persisted")
        .collect::<Vec<_>>();
    let observed_at = persisted.last().map(|point| point.recorded_at.clone());
    let percentage = numeric_history_summary(
        persisted.iter().filter_map(|point| {
            point
                .metrics
                .percentage
                .value
                .map(|value| (value, point.recorded_at.clone()))
        }),
        observed_at.clone(),
    );
    let power_watts = numeric_history_summary(
        persisted.iter().filter_map(|point| {
            point
                .metrics
                .power_watts
                .value
                .map(|value| (value, point.recorded_at.clone()))
        }),
        observed_at.clone(),
    );
    let energy_now_wh = numeric_history_summary(
        persisted.iter().filter_map(|point| {
            point
                .metrics
                .energy_now_wh
                .value
                .map(|value| (value, point.recorded_at.clone()))
        }),
        observed_at.clone(),
    );
    let energy_values = persisted
        .iter()
        .filter_map(|point| point.metrics.energy_now_wh.value)
        .collect::<Vec<_>>();
    let observed_energy_wh = if energy_values.len() >= 2 && gaps.is_empty() {
        let first = energy_values[0];
        let last = *energy_values.last().expect("length is at least two");
        ObservedEnergySummary {
            first: Some(first),
            last: Some(last),
            change: Some(last - first),
            observed_samples: energy_values.len(),
            source: "derived",
            availability: "available",
            observed_at,
        }
    } else {
        unavailable_observed_energy(energy_values.len())
    };

    RecentBatteryHistorySummary {
        percentage,
        power_watts,
        energy_now_wh,
        observed_energy_wh,
    }
}

fn numeric_history_summary(
    values: impl IntoIterator<Item = (f64, String)>,
    observed_at: Option<String>,
) -> NumericHistorySummary {
    let values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() {
        return unavailable_numeric_summary(0);
    }
    // The caller always supplies values in chronological order, so folding
    // with a strict `<`/`>` comparison keeps the *first* recorded instant an
    // extremum was reached, which is the most useful and most deterministic
    // answer when a value is repeated across several consecutive samples.
    let (minimum, minimum_at) = values
        .iter()
        .fold(None, |acc: Option<&(f64, String)>, candidate| match acc {
            Some(current) if candidate.0 >= current.0 => acc,
            _ => Some(candidate),
        })
        .map_or((None, None), |(value, at)| (Some(*value), Some(at.clone())));
    let (maximum, maximum_at) = values
        .iter()
        .fold(None, |acc: Option<&(f64, String)>, candidate| match acc {
            Some(current) if candidate.0 <= current.0 => acc,
            _ => Some(candidate),
        })
        .map_or((None, None), |(value, at)| (Some(*value), Some(at.clone())));
    let count = u32::try_from(values.len()).expect("history chart point count fits in u32");
    let average = values.iter().map(|(value, _)| value).sum::<f64>() / f64::from(count);
    NumericHistorySummary {
        minimum,
        maximum,
        average: Some(average),
        minimum_at,
        maximum_at,
        observed_samples: values.len(),
        source: "derived",
        availability: "available",
        observed_at,
    }
}

fn empty_history_summary() -> RecentBatteryHistorySummary {
    RecentBatteryHistorySummary {
        percentage: unavailable_numeric_summary(0),
        power_watts: unavailable_numeric_summary(0),
        energy_now_wh: unavailable_numeric_summary(0),
        observed_energy_wh: unavailable_observed_energy(0),
    }
}

fn unavailable_numeric_summary(observed_samples: usize) -> NumericHistorySummary {
    NumericHistorySummary {
        minimum: None,
        maximum: None,
        average: None,
        minimum_at: None,
        maximum_at: None,
        observed_samples,
        source: "unavailable",
        availability: "unavailable",
        observed_at: None,
    }
}

fn unavailable_observed_energy(observed_samples: usize) -> ObservedEnergySummary {
    ObservedEnergySummary {
        first: None,
        last: None,
        change: None,
        observed_samples,
        source: "unavailable",
        availability: "unavailable",
        observed_at: None,
    }
}

fn history_freshness(points: &[RecentHistoryPoint], now: OffsetDateTime) -> &'static str {
    let Some(latest_persisted) = points
        .iter()
        .filter(|point| point.kind == "persisted")
        .map(|point| point.recorded_at.as_str())
        .max()
    else {
        return if points.iter().any(|point| point.kind == "transient") {
            "fresh"
        } else {
            "unknown"
        };
    };
    if OffsetDateTime::parse(latest_persisted, &Rfc3339)
        .is_ok_and(|timestamp| now - timestamp > time::Duration::minutes(3))
    {
        "stale"
    } else {
        "fresh"
    }
}

fn aggregate_metric_status(value: Option<f64>) -> (&'static str, &'static str) {
    if value.is_some() {
        ("derived", "available")
    } else {
        ("unavailable", "unavailable")
    }
}

fn recorder_state() -> &'static str {
    match SystemdUserScheduler::for_current_user().status() {
        SchedulerStatus::Enabled => "enabled",
        SchedulerStatus::Disabled => "disabled",
        SchedulerStatus::Unavailable { .. } => "unsupported",
    }
}

fn recorder_unavailable_reason(recorder_state: &str) -> &'static str {
    match recorder_state {
        "enabled" => "no-recorded-samples",
        "disabled" => "recorder-disabled",
        "unsupported" => "unsupported",
        _ => "unknown",
    }
}

fn storage_metric_source(source: storage::MetricSource) -> &'static str {
    match source {
        storage::MetricSource::Upower => "upower",
        storage::MetricSource::Sysfs => "sysfs",
        storage::MetricSource::Derived => "derived",
        storage::MetricSource::Unavailable => "unavailable",
    }
}

fn storage_state(state: storage::SampleState) -> &'static str {
    match state {
        storage::SampleState::Charging => "charging",
        storage::SampleState::Discharging => "discharging",
        storage::SampleState::Full => "full",
        storage::SampleState::Idle => "idle",
        storage::SampleState::Unknown => "unknown",
    }
}

/// Returns the opt-in background recorder state without creating a database.
#[tauri::command]
fn get_recorder_status() -> RecorderStatusResponse {
    recorder_status(None)
}

/// Explicitly enables or disables background recording for the current user.
///
/// Enabling stages the recorder and its systemd user units under XDG paths,
/// then asks the existing user manager to enable the timer. Disabling preserves
/// both the units and history while stopping future samples.
#[tauri::command]
fn set_recorder_enabled(enabled: bool) -> RecorderStatusResponse {
    let scheduler = SystemdUserScheduler::for_current_user();
    if enabled {
        if let SchedulerStatus::Unavailable { reason } = scheduler.status() {
            return recorder_status(Some(format!(
                "background recording is unsupported on this system: {reason}"
            )));
        }
    }

    let result = if enabled {
        recorder_install::stage_built_recorder().and_then(|_| {
            scheduler.enable().map_err(|error| {
                recorder_install::RecorderInstallError::Io(std::io::Error::other(error))
            })
        })
    } else {
        scheduler.disable().map_err(|error| {
            recorder_install::RecorderInstallError::Io(std::io::Error::other(error))
        })
    };

    match result {
        Ok(()) => recorder_status(None),
        Err(error) => recorder_status(Some(error.to_string())),
    }
}

/// The stable frontend contract for recorder control and diagnostics.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecorderStatusResponse {
    schema_version: u8,
    supported: bool,
    enabled: bool,
    transition: &'static str,
    health: &'static str,
    last_recorded_at: Option<String>,
    error: Option<String>,
}

fn recorder_status(request_error: Option<String>) -> RecorderStatusResponse {
    let scheduler_status = SystemdUserScheduler::for_current_user().status();
    let (supported, enabled, scheduler_error) = match scheduler_status {
        SchedulerStatus::Enabled => (true, true, None),
        SchedulerStatus::Disabled => (true, false, None),
        SchedulerStatus::Unavailable { reason } => (false, false, Some(reason)),
    };

    if !supported {
        return RecorderStatusResponse {
            schema_version: 1,
            supported: false,
            enabled: false,
            transition: "idle",
            health: "unknown",
            last_recorded_at: None,
            error: request_error.or(scheduler_error),
        };
    }

    match storage::last_recorded_at_if_exists() {
        Ok(last_recorded_at) => RecorderStatusResponse {
            schema_version: 1,
            supported: true,
            enabled,
            transition: "idle",
            health: if enabled && last_recorded_at.is_some() {
                "healthy"
            } else {
                "unknown"
            },
            last_recorded_at: last_recorded_at.and_then(format_timestamp),
            error: request_error,
        },
        Err(error) => RecorderStatusResponse {
            schema_version: 1,
            supported: true,
            enabled,
            transition: "idle",
            health: "error",
            last_recorded_at: None,
            error: request_error.or_else(|| Some(error.to_string())),
        },
    }
}

fn format_timestamp(timestamp: OffsetDateTime) -> Option<String> {
    timestamp
        .to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .ok()
}

/// Creates the desktop application builder.
fn app_builder() -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default().invoke_handler(tauri::generate_handler![
        get_battery_dashboard,
        get_recent_battery_history,
        get_battery_health,
        get_battery_anomalies,
        get_power_profile,
        set_power_profile,
        get_battery_session_history,
        rebuild_battery_session_history,
        get_today_vs_yesterday_usage,
        get_battery_life_estimate,
        get_runtime_forecast,
        export_battery_history,
        get_recorder_status,
        set_recorder_enabled
    ])
}

fn main() {
    app_builder()
        .run(tauri::generate_context!())
        .expect("failed to run Battery Dashboard");
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss, clippy::float_cmp)]
mod tests {
    use super::{
        ExportRequest, aggregate_persisted_history, app_builder, combine_battery_day_usage,
        day_usage_dto, export_battery_history, history_summary_from_points, local_day_bounds,
        persisted_history_point, supported_history_range,
    };
    use crate::storage::{
        HistoryAvailability, HistoryFreshness, HistoryMetric, HistoryMetrics, HistorySample,
        MetricSource, NewBatterySample, SampleMetric, SampleMetrics, SampleState, Storage,
        database_path_from_data_home,
    };

    fn unavailable_metric() -> HistoryMetric {
        HistoryMetric {
            value: None,
            source: MetricSource::Unavailable,
            availability: HistoryAvailability::Unavailable,
            freshness: HistoryFreshness::Recorded,
        }
    }

    fn available_metric(value: f64) -> HistoryMetric {
        HistoryMetric {
            value: Some(value),
            source: MetricSource::Upower,
            availability: HistoryAvailability::Available,
            freshness: HistoryFreshness::Recorded,
        }
    }

    fn sample_with_percentage(
        battery_id: &str,
        recorded_at: &str,
        percentage: Option<f64>,
    ) -> HistorySample {
        HistorySample {
            battery_id: battery_id.to_owned(),
            recorded_at: recorded_at.to_owned(),
            boot_id: "boot".to_owned(),
            boot_seconds: 0.0,
            state: SampleState::Discharging,
            metrics: HistoryMetrics {
                percentage: percentage.map_or_else(unavailable_metric, available_metric),
                energy_now_wh: percentage
                    .map_or_else(unavailable_metric, |_| available_metric(40.0)),
                energy_full_wh: percentage
                    .map_or_else(unavailable_metric, |_| available_metric(50.0)),
                energy_design_wh: unavailable_metric(),
                power_watts: unavailable_metric(),
                voltage_volts: unavailable_metric(),
                current_amps: unavailable_metric(),
                temperature_celsius: unavailable_metric(),
                time_remaining_minutes: unavailable_metric(),
                cycle_count: unavailable_metric(),
            },
        }
    }

    #[test]
    fn aggregate_history_ignores_a_battery_that_never_reports_a_percentage() {
        // A discovered power-supply battery that never yields a usable
        // percentage (for example a paired wireless mouse's hidpp battery)
        // must not permanently block the aggregate percentage series or
        // manufacture a "missing-samples" gap at every other instant.
        let noise = sample_with_percentage("hidpp_battery_0", "2026-08-23T17:07:00Z", None);
        let real_samples = [
            sample_with_percentage("BAT0", "2026-08-23T17:07:00Z", Some(80.0)),
            sample_with_percentage("BAT0", "2026-08-23T17:08:00Z", Some(79.0)),
            sample_with_percentage("BAT0", "2026-08-23T17:09:00Z", Some(78.0)),
        ];
        let samples = [&noise, &real_samples[0], &real_samples[1], &real_samples[2]];

        let mapped = aggregate_persisted_history(&samples);

        assert!(
            mapped.gaps.is_empty(),
            "a battery with no usable readings must not manufacture aggregate gaps: {:?}",
            mapped
                .gaps
                .iter()
                .map(|gap| &gap.starts_at)
                .collect::<Vec<_>>()
        );
        assert_eq!(mapped.points.len(), 3);
        let available_points = mapped
            .points
            .iter()
            .filter(|point| point.metrics.percentage.value.is_some())
            .count();
        assert_eq!(
            available_points, 3,
            "the aggregate percentage must still compute once the non-contributing battery is excluded"
        );
    }

    #[test]
    fn aggregate_history_still_reports_a_gap_between_two_real_batteries() {
        // Two batteries that both regularly contribute usable readings still
        // produce a real "missing-samples" gap when one is absent at a shared
        // instant.
        let samples = [
            sample_with_percentage("BAT0", "2026-08-23T17:07:00Z", Some(80.0)),
            sample_with_percentage("BAT1", "2026-08-23T17:07:00Z", Some(60.0)),
            sample_with_percentage("BAT0", "2026-08-23T17:08:00Z", Some(79.0)),
        ];
        let refs = [&samples[0], &samples[1], &samples[2]];

        let mapped = aggregate_persisted_history(&refs);

        assert_eq!(mapped.gaps.len(), 1);
        assert_eq!(mapped.gaps[0].starts_at, "2026-08-23T17:08:00Z");
    }

    #[test]
    fn numeric_history_summary_reports_when_the_minimum_and_maximum_were_recorded() {
        // The user reasonably expects to know *when* an extremum happened,
        // not only what it was. `observed_at` alone (the last sample time)
        // cannot answer that: the minimum and maximum can occur anywhere in
        // the window, so each needs its own recorded timestamp.
        let samples = [
            sample_with_percentage("BAT0", "2026-08-23T17:00:00Z", Some(80.0)),
            sample_with_percentage("BAT0", "2026-08-23T17:01:00Z", Some(60.0)),
            sample_with_percentage("BAT0", "2026-08-23T17:02:00Z", Some(90.0)),
            sample_with_percentage("BAT0", "2026-08-23T17:03:00Z", Some(70.0)),
        ];
        let points = samples
            .iter()
            .map(persisted_history_point)
            .collect::<Vec<_>>();

        let summary = history_summary_from_points(&points, &[]);

        assert_eq!(summary.percentage.minimum, Some(60.0));
        assert_eq!(
            summary.percentage.minimum_at.as_deref(),
            Some("2026-08-23T17:01:00Z")
        );
        assert_eq!(summary.percentage.maximum, Some(90.0));
        assert_eq!(
            summary.percentage.maximum_at.as_deref(),
            Some("2026-08-23T17:02:00Z")
        );
        // The overall `observed_at` remains the last sample time, distinct
        // from either extremum's own timestamp.
        assert_eq!(
            summary.percentage.observed_at.as_deref(),
            Some("2026-08-23T17:03:00Z")
        );
    }

    #[test]
    fn numeric_history_summary_picks_the_first_occurrence_of_a_repeated_extremum() {
        // When the same extreme value is recorded more than once, the first
        // recorded instant is the most useful and deterministic answer.
        let samples = [
            sample_with_percentage("BAT0", "2026-08-23T17:00:00Z", Some(50.0)),
            sample_with_percentage("BAT0", "2026-08-23T17:01:00Z", Some(50.0)),
        ];
        let points = samples
            .iter()
            .map(persisted_history_point)
            .collect::<Vec<_>>();

        let summary = history_summary_from_points(&points, &[]);

        assert_eq!(
            summary.percentage.minimum_at.as_deref(),
            Some("2026-08-23T17:00:00Z")
        );
        assert_eq!(
            summary.percentage.maximum_at.as_deref(),
            Some("2026-08-23T17:00:00Z")
        );
    }

    #[test]
    fn desktop_builder_can_be_created_without_hardware_access() {
        let _builder = app_builder();
    }

    #[test]
    fn export_command_requires_an_absolute_user_destination() {
        let response = export_battery_history(ExportRequest {
            data_type: "raw-samples".to_owned(),
            format: "csv".to_owned(),
            destination: "history.csv".to_owned(),
            battery_id: None,
            start_date: None,
            end_date: None,
            timezone: None,
            summary_period: None,
        });
        assert_eq!(response.availability, "unavailable");
        assert_eq!(response.unavailable_reason, Some("invalid-request"));
        assert!(response.error.is_some());
    }

    fn temporary_storage_path(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the system clock is after UNIX epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "battery-dashboard-main-{name}-{nanos}-{}",
            std::process::id()
        ));
        database_path_from_data_home(root)
    }

    fn day_usage_sample(
        battery_id: &str,
        minute: i64,
        percentage: f64,
        energy_full_wh: f64,
        power_watts: f64,
        state: SampleState,
    ) -> NewBatterySample {
        let available = |value: f64| SampleMetric {
            value: Some(value),
            source: MetricSource::Sysfs,
        };
        NewBatterySample {
            battery_id: battery_id.to_owned(),
            recorded_at: time::macros::datetime!(2026-08-23 00:05 UTC)
                + time::Duration::minutes(minute),
            boot_id: "11111111-2222-3333-4444-555555555555".to_owned(),
            boot_seconds: 1000.0 + minute as f64 * 60.0,
            state,
            metrics: SampleMetrics {
                percentage: available(percentage),
                energy_now_wh: available(percentage / 100.0 * energy_full_wh),
                energy_full_wh: available(energy_full_wh),
                energy_design_wh: SampleMetric::unavailable(),
                power_watts: available(power_watts),
                voltage_volts: available(12.0),
                current_amps: SampleMetric::unavailable(),
                temperature_celsius: SampleMetric::unavailable(),
                time_remaining_minutes: SampleMetric::unavailable(),
                cycle_count: SampleMetric::unavailable(),
            },
        }
    }

    fn day_bounds() -> (time::OffsetDateTime, time::OffsetDateTime) {
        (
            time::macros::datetime!(2026-08-23 00:00 UTC),
            time::macros::datetime!(2026-08-24 00:00 UTC),
        )
    }

    #[test]
    fn local_day_bounds_reflects_a_spring_forward_dst_transition() {
        // 2026-03-29 is the last Sunday of March: the EU clocks jump from
        // 02:00 to 03:00 CET/CEST, so the wall-clock day is only 23 hours.
        // This must come from the same IANA/DST-aware midnight resolution
        // used for session history, not a fixed 24-hour assumption.
        let timezone: chrono_tz::Tz = "Europe/Rome".parse().expect("valid IANA timezone");
        let day = chrono::NaiveDate::from_ymd_opt(2026, 3, 29).expect("valid date");

        let (start, end) = local_day_bounds(timezone, day).expect("bounds resolve");

        assert_eq!((end - start).whole_seconds(), 23 * 3600);
    }

    #[test]
    fn local_day_bounds_reflects_a_fall_back_dst_transition() {
        // 2026-10-25 is the last Sunday of October: clocks fall back from
        // 03:00 to 02:00 CEST/CET, so the wall-clock day is 25 hours.
        let timezone: chrono_tz::Tz = "Europe/Rome".parse().expect("valid IANA timezone");
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 25).expect("valid date");

        let (start, end) = local_day_bounds(timezone, day).expect("bounds resolve");

        assert_eq!((end - start).whole_seconds(), 25 * 3600);
    }

    #[test]
    fn day_usage_dto_reports_insufficient_evidence_for_a_battery_with_no_samples() {
        let path = temporary_storage_path("no-samples");
        let storage = Storage::open_at(&path).expect("database opens");
        let (start, end) = day_bounds();

        let dto = day_usage_dto(
            &storage,
            Some("BAT0"),
            chrono::NaiveDate::from_ymd_opt(2026, 8, 23).expect("valid date"),
            start,
            end,
        )
        .expect("a summary is always produced for a valid range");

        assert!(dto.available);
        assert_eq!(dto.evidence, "insufficient");
        assert_eq!(dto.insufficient_reason, Some("no-recording"));
        assert_eq!(dto.sample_count, 0);
        assert_eq!(dto.percentage_change, None);
        assert_eq!(dto.energy_change_wh, None);
        drop(storage);
        std::fs::remove_dir_all(path.parent().expect("database has a parent directory"))
            .expect("test directory is removable");
    }

    #[test]
    fn combine_battery_day_usage_excludes_batteries_below_the_evidence_policy() {
        let path = temporary_storage_path("combine");
        let mut storage = Storage::open_at(&path).expect("database opens");
        let (start, end) = day_bounds();
        for minute in 0_i64..=11 {
            storage
                .insert_sample(&day_usage_sample(
                    "BAT0",
                    minute,
                    80.0 - minute as f64,
                    50.0,
                    -10.0,
                    SampleState::Discharging,
                ))
                .expect("sample inserts");
        }
        // BAT1 only has two samples: far below the evidence policy, and must
        // never be treated as a zero contribution.
        for minute in 0_i64..2 {
            storage
                .insert_sample(&day_usage_sample(
                    "BAT1",
                    minute,
                    40.0,
                    20.0,
                    5.0,
                    SampleState::Charging,
                ))
                .expect("sample inserts");
        }

        let (percentage_change, discharge_power, charge_power, contributing) =
            combine_battery_day_usage(
                &storage,
                &["BAT0".to_owned(), "BAT1".to_owned()],
                start,
                end,
            );

        assert_eq!(contributing, Some(1));
        assert_eq!(percentage_change, Some(-11.0));
        assert_eq!(discharge_power, Some(10.0));
        assert_eq!(charge_power, None);
        drop(storage);
        std::fs::remove_dir_all(path.parent().expect("database has a parent directory"))
            .expect("test directory is removable");
    }

    #[test]
    fn combine_battery_day_usage_weights_percentage_change_by_full_capacity() {
        let path = temporary_storage_path("combine-weighted");
        let mut storage = Storage::open_at(&path).expect("database opens");
        let (start, end) = day_bounds();
        for minute in 0_i64..=11 {
            storage
                .insert_sample(&day_usage_sample(
                    "BAT0",
                    minute,
                    80.0 - minute as f64, // -11 points over a 60 Wh battery
                    60.0,
                    -10.0,
                    SampleState::Discharging,
                ))
                .expect("sample inserts");
            storage
                .insert_sample(&day_usage_sample(
                    "BAT1",
                    minute,
                    50.0 + minute as f64, // +11 points over a 20 Wh battery
                    20.0,
                    8.0,
                    SampleState::Charging,
                ))
                .expect("sample inserts");
        }

        let (percentage_change, discharge_power, charge_power, contributing) =
            combine_battery_day_usage(
                &storage,
                &["BAT0".to_owned(), "BAT1".to_owned()],
                start,
                end,
            );

        assert_eq!(contributing, Some(2));
        // Weighted by full capacity: (-11 * 60 + 11 * 20) / (60 + 20) = -5.5.
        let percentage_change = percentage_change.expect("both batteries have sufficient evidence");
        assert!((percentage_change - (-5.5)).abs() < 0.001);
        assert_eq!(discharge_power, Some(10.0));
        assert_eq!(charge_power, Some(8.0));
        drop(storage);
        std::fs::remove_dir_all(path.parent().expect("database has a parent directory"))
            .expect("test directory is removable");
    }

    #[test]
    fn supported_history_range_accepts_every_dashboard_window() {
        for hours in [2, 6, 12, 24, 72, 168, 720] {
            assert_eq!(supported_history_range(hours), Some(hours));
        }
    }

    #[test]
    fn supported_history_range_rejects_unlisted_windows() {
        for hours in [0, 1, 3, 48, 96, 200, 500, 1000] {
            assert_eq!(supported_history_range(hours), None);
        }
    }
}
