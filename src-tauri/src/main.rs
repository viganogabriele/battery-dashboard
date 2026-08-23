//! Native desktop entry point for Battery Dashboard.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use battery_dashboard_desktop::{
    battery, recorder_install,
    scheduler::{SchedulerStatus, SystemdUserScheduler},
    storage,
};
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
