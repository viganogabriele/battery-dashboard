//! Native desktop entry point for Battery Dashboard.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use battery_dashboard_desktop::{
    battery, recorder_install,
    scheduler::{SchedulerStatus, SystemdUserScheduler},
    storage,
};
use chrono::{Datelike, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Serialize;
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
    range_hours: u8,
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
    let from_chrono = |value: chrono::DateTime<Utc>| {
        OffsetDateTime::from_unix_timestamp(value.timestamp())
            .map_err(|_| ())
            .map(|value| value + time::Duration::nanoseconds(i64::from(value.nanosecond())))
    };
    Ok((from_chrono(start)?, from_chrono(end)?))
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
        entry.coverage_seconds = match (entry.coverage_seconds, session.observed_duration_seconds) {
            (Some(total), Some(value)) => Some(total + value),
            _ => None,
        };
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
    range_hours: u8,
    collected_at: Option<String>,
    points: Vec<RecentHistoryPoint>,
    gaps: Vec<RecentHistoryGap>,
    summary: RecentBatteryHistorySummary,
}

fn supported_history_range(range_hours: u8) -> Option<u8> {
    matches!(range_hours, 2 | 6 | 12 | 24).then_some(range_hours)
}

fn unavailable_history(
    battery_id: Option<String>,
    range_hours: u8,
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
        persisted
            .iter()
            .filter_map(|point| point.metrics.percentage.value),
        observed_at.clone(),
    );
    let power_watts = numeric_history_summary(
        persisted
            .iter()
            .filter_map(|point| point.metrics.power_watts.value),
        observed_at.clone(),
    );
    let energy_now_wh = numeric_history_summary(
        persisted
            .iter()
            .filter_map(|point| point.metrics.energy_now_wh.value),
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
    values: impl IntoIterator<Item = f64>,
    observed_at: Option<String>,
) -> NumericHistorySummary {
    let values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() {
        return unavailable_numeric_summary(0);
    }
    let minimum = values.iter().copied().reduce(f64::min);
    let maximum = values.iter().copied().reduce(f64::max);
    let count = u32::try_from(values.len()).expect("history chart point count fits in u32");
    let average = values.iter().sum::<f64>() / f64::from(count);
    NumericHistorySummary {
        minimum,
        maximum,
        average: Some(average),
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
        .ok()
        .is_some_and(|timestamp| now - timestamp > time::Duration::minutes(3))
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
        get_battery_session_history,
        rebuild_battery_session_history,
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
mod tests {
    use super::app_builder;

    #[test]
    fn desktop_builder_can_be_created_without_hardware_access() {
        let _builder = app_builder();
    }
}
