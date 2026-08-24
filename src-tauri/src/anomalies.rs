//! Conservative, local anomaly detection over recorded battery samples.
//!
//! Every result in this module is derived from an observed sample or an
//! observed contiguous pair of samples.  Gaps, missing metrics, reboot
//! boundaries, and mixed batteries are never filled or combined.

use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::storage::{HistorySample, HistoryTimelineItem, SampleState};

const MIN_BASELINE_VALUES: usize = 5;
const MAX_ANOMALIES: usize = 100;
const CONTIGUOUS_LIMIT_SECONDS: f64 = 180.0;
const MAD_SCALE: f64 = 1.4826;

/// A kind of evidence-backed historical anomaly.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnomalyKind {
    /// A discharging sample is far outside the observed draw baseline.
    UnusualPower,
    /// A contiguous discharge interval loses charge faster than its baseline.
    RapidDischarge,
    /// A contiguous charging run stops before a full state is observed.
    InterruptedCharge,
}

/// One anomaly tied to one or two recorded observations.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryAnomaly {
    /// Classification of the observation.
    pub kind: AnomalyKind,
    /// Timestamp of the sample or end of the pair that supplied the evidence.
    pub recorded_at: String,
    /// Start of a pair-based observation, when applicable.
    pub started_at: Option<String>,
    /// Human-readable severity based on the observed deviation.
    pub severity: &'static str,
    /// Confidence based on baseline size and robust deviation.
    pub confidence: &'static str,
    /// Observed value in `unit`.
    pub observed_value: Option<f64>,
    /// Robust observed baseline in `unit`, when one exists.
    pub baseline_value: Option<f64>,
    /// Unit for the numeric fields.
    pub unit: &'static str,
    /// Explanation containing only observed/baseline facts.
    pub explanation: String,
}

/// Why an anomaly report cannot yet be conclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InsufficiencyReason {
    /// The history has too few samples for a baseline.
    TooFewSamples,
    /// Samples exist, but required fields are unavailable.
    NoUsableMetrics,
    /// There are not enough comparable observations for a robust baseline.
    TooFewBaselineValues,
}

impl InsufficiencyReason {
    /// Returns the stable response spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TooFewSamples => "too-few-samples",
            Self::NoUsableMetrics => "no-usable-metrics",
            Self::TooFewBaselineValues => "too-few-baseline-values",
        }
    }
}

/// Pure analysis output.  Database and request failures are represented by
/// the command layer, while this report represents actual sample adequacy.
#[derive(Clone, Debug, PartialEq)]
pub struct AnomalyReport {
    /// `available` when a detector had enough evidence, otherwise
    /// `insufficient`.
    pub availability: &'static str,
    /// Why the report is insufficient, if applicable.
    pub insufficiency_reason: Option<InsufficiencyReason>,
    /// Number of durable samples considered.
    pub observed_samples: usize,
    /// Number of usable discharging power observations.
    pub power_samples: usize,
    /// Number of usable contiguous discharge intervals.
    pub discharge_intervals: usize,
    /// Number of charging-to-non-full transitions considered.
    pub charging_transitions: usize,
    /// Evidence-backed anomalies, possibly empty for a stable history.
    pub anomalies: Vec<BatteryAnomaly>,
}

/// Analyzes a history timeline while preserving its explicit gap boundaries.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn analyze(timeline: &[HistoryTimelineItem]) -> AnomalyReport {
    let segments = segments(timeline);
    let observed_samples = segments.iter().map(Vec::len).sum();
    if observed_samples < 2 {
        return insufficient(
            observed_samples,
            0,
            0,
            0,
            InsufficiencyReason::TooFewSamples,
        );
    }

    let mut power_observations = Vec::new();
    let mut discharge_intervals = Vec::new();
    let mut interrupted = Vec::new();
    for segment in &segments {
        for sample in segment.iter().copied() {
            if sample.state == SampleState::Discharging {
                // Power is signed in storage (negative while discharging),
                // but absolute draw remains robust to provider sign details.
                if let Some(value) = sample
                    .metrics
                    .power_watts
                    .value
                    .filter(|value| value.is_finite())
                {
                    power_observations.push((sample, value.abs()));
                }
            }
        }
        for pair in segment.windows(2) {
            let [first, second] = pair else { continue };
            let first = *first;
            let second = *second;
            if let Some(interval) = contiguous_interval(first, second) {
                if first.state == SampleState::Discharging
                    && second.state == SampleState::Discharging
                {
                    if let Some(drop) = percentage_drop(first, second, interval) {
                        discharge_intervals.push(DropObservation {
                            first,
                            second,
                            value: drop,
                            unit: "%/h",
                        });
                    } else if let Some(drop) = energy_drop(first, second, interval) {
                        discharge_intervals.push(DropObservation {
                            first,
                            second,
                            value: drop,
                            unit: "Wh/h",
                        });
                    }
                }
                if first.state == SampleState::Charging
                    && second.state != SampleState::Charging
                    && second.state != SampleState::Full
                {
                    // Count only a complete, observed charging run.  A run
                    // ending at a query boundary has no transition evidence.
                    let run_len = segment
                        .iter()
                        .rev()
                        .skip_while(|sample| sample.recorded_at != first.recorded_at)
                        .take_while(|sample| sample.state == SampleState::Charging)
                        .count();
                    if run_len >= 3
                        && first
                            .metrics
                            .percentage
                            .value
                            .is_some_and(|value| value.is_finite() && value < 99.0)
                    {
                        interrupted.push((first, second, run_len));
                    }
                }
            }
        }
    }

    let mut anomalies = unusual_power(&power_observations);
    anomalies.extend(rapid_discharge(&discharge_intervals));
    anomalies.extend(interrupted_charge(&interrupted));
    anomalies.truncate(MAX_ANOMALIES);
    anomalies.sort_by(|left, right| {
        left.recorded_at
            .cmp(&right.recorded_at)
            .then_with(|| kind_order(left.kind).cmp(&kind_order(right.kind)))
    });

    let usable_detector = power_observations.len() >= MIN_BASELINE_VALUES
        || discharge_intervals.len() >= MIN_BASELINE_VALUES
        || !interrupted.is_empty();
    let availability = if usable_detector {
        "available"
    } else {
        "insufficient"
    };
    let insufficiency_reason = (!usable_detector).then_some({
        if power_observations.is_empty() && discharge_intervals.is_empty() {
            InsufficiencyReason::NoUsableMetrics
        } else {
            InsufficiencyReason::TooFewBaselineValues
        }
    });
    AnomalyReport {
        availability,
        insufficiency_reason,
        observed_samples,
        power_samples: power_observations.len(),
        discharge_intervals: discharge_intervals.len(),
        charging_transitions: interrupted.len(),
        anomalies,
    }
}

struct DropObservation<'a> {
    first: &'a HistorySample,
    second: &'a HistorySample,
    value: f64,
    unit: &'static str,
}

fn insufficient(
    observed_samples: usize,
    power_samples: usize,
    discharge_intervals: usize,
    charging_transitions: usize,
    reason: InsufficiencyReason,
) -> AnomalyReport {
    AnomalyReport {
        availability: "insufficient",
        insufficiency_reason: Some(reason),
        observed_samples,
        power_samples,
        discharge_intervals,
        charging_transitions,
        anomalies: Vec::new(),
    }
}

fn segments(timeline: &[HistoryTimelineItem]) -> Vec<Vec<&HistorySample>> {
    let mut result = Vec::new();
    let mut current = Vec::new();
    let mut battery_id: Option<&str> = None;
    for item in timeline {
        let HistoryTimelineItem::Sample(sample) = item else {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
            battery_id = None;
            continue;
        };
        let sample = sample.as_ref();
        if battery_id.is_some_and(|id| id != sample.battery_id) && !current.is_empty() {
            result.push(std::mem::take(&mut current));
        }
        battery_id = Some(&sample.battery_id);
        current.push(sample);
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

fn unusual_power(observations: &[(&HistorySample, f64)]) -> Vec<BatteryAnomaly> {
    if observations.len() < MIN_BASELINE_VALUES {
        return Vec::new();
    }
    let values = observations
        .iter()
        .map(|(_, value)| *value)
        .collect::<Vec<_>>();
    let (median, scale) = robust_baseline(&values);
    let threshold = median + scale.max((median * 0.5).max(1.0));
    observations
        .iter()
        .filter_map(|(sample, value)| {
            if *value <= threshold {
                return None;
            }
            let excess = *value - threshold;
            let confidence = confidence(excess, scale);
            Some(BatteryAnomaly {
                kind: AnomalyKind::UnusualPower,
                recorded_at: sample.recorded_at.clone(),
                started_at: None,
                severity: if *value > threshold * 2.0 {
                    "high"
                } else {
                    "medium"
                },
                confidence,
                observed_value: Some(*value),
                baseline_value: Some(median),
                unit: "W",
                explanation: format!("observed discharging draw {value:.2} W exceeded the historical baseline {median:.2} W"),
            })
        })
        .collect()
}

fn rapid_discharge(observations: &[DropObservation<'_>]) -> Vec<BatteryAnomaly> {
    if observations.len() < MIN_BASELINE_VALUES {
        return Vec::new();
    }
    let unit = observations[0].unit;
    let same_unit = observations
        .iter()
        .all(|observation| observation.unit == unit);
    if !same_unit {
        return Vec::new();
    }
    let values = observations
        .iter()
        .map(|observation| observation.value)
        .collect::<Vec<_>>();
    let (median, scale) = robust_baseline(&values);
    let floor = 1.0;
    let threshold = median + scale.max((median * 0.5).max(floor));
    observations
        .iter()
        .filter_map(|observation| {
            if observation.value <= threshold {
                return None;
            }
            let excess = observation.value - threshold;
            Some(BatteryAnomaly {
                kind: AnomalyKind::RapidDischarge,
                recorded_at: observation.second.recorded_at.clone(),
                started_at: Some(observation.first.recorded_at.clone()),
                severity: if observation.value > threshold * 2.0 {
                    "high"
                } else {
                    "medium"
                },
                confidence: confidence(excess, scale),
                observed_value: Some(observation.value),
                baseline_value: Some(median),
                unit,
                explanation: format!(
                    "observed discharge loss {:.2} {} exceeded the historical baseline {:.2} {}",
                    observation.value, unit, median, unit
                ),
            })
        })
        .collect()
}

fn interrupted_charge(
    observations: &[(&HistorySample, &HistorySample, usize)],
) -> Vec<BatteryAnomaly> {
    observations
        .iter()
        .map(|(first, second, run_len)| BatteryAnomaly {
            kind: AnomalyKind::InterruptedCharge,
            recorded_at: second.recorded_at.clone(),
            started_at: Some(first.recorded_at.clone()),
            severity: "medium",
            confidence: if *run_len >= 6 { "high" } else { "medium" },
            observed_value: first.metrics.percentage.value,
            baseline_value: None,
            unit: "%",
            explanation: format!(
                "charging stopped at {:.1}% after {run_len} contiguous recorded charging samples; no full state was observed",
                first.metrics.percentage.value.unwrap_or_default()
            ),
        })
        .collect()
}

fn robust_baseline(values: &[f64]) -> (f64, f64) {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = median_sorted(&sorted);
    let mut deviations = sorted
        .iter()
        .map(|value| (value - median).abs())
        .collect::<Vec<_>>();
    deviations.sort_by(f64::total_cmp);
    (median, median_sorted(&deviations) * MAD_SCALE)
}

fn median_sorted(values: &[f64]) -> f64 {
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        values[middle - 1].midpoint(values[middle])
    } else {
        values[middle]
    }
}

fn confidence(excess: f64, scale: f64) -> &'static str {
    if scale > 0.0 && excess / scale >= 3.0 {
        "high"
    } else {
        "medium"
    }
}

fn kind_order(kind: AnomalyKind) -> u8 {
    match kind {
        AnomalyKind::UnusualPower => 0,
        AnomalyKind::RapidDischarge => 1,
        AnomalyKind::InterruptedCharge => 2,
    }
}

fn contiguous_interval(first: &HistorySample, second: &HistorySample) -> Option<f64> {
    if first.boot_id != second.boot_id {
        return None;
    }
    let first_time = OffsetDateTime::parse(&first.recorded_at, &Rfc3339).ok()?;
    let second_time = OffsetDateTime::parse(&second.recorded_at, &Rfc3339).ok()?;
    let wall_seconds = (second_time - first_time).as_seconds_f64();
    let boot_seconds = second.boot_seconds - first.boot_seconds;
    (wall_seconds.is_finite()
        && boot_seconds.is_finite()
        && wall_seconds > 0.0
        && boot_seconds > 0.0
        && wall_seconds <= CONTIGUOUS_LIMIT_SECONDS
        && boot_seconds <= CONTIGUOUS_LIMIT_SECONDS)
        .then_some(boot_seconds)
}

fn percentage_drop(first: &HistorySample, second: &HistorySample, seconds: f64) -> Option<f64> {
    let start = first.metrics.percentage.value?;
    let end = second.metrics.percentage.value?;
    let drop = start - end;
    (drop.is_finite() && drop > 0.0).then_some(drop * 3_600.0 / seconds)
}

fn energy_drop(first: &HistorySample, second: &HistorySample, seconds: f64) -> Option<f64> {
    let start = first.metrics.energy_now_wh.value?;
    let end = second.metrics.energy_now_wh.value?;
    let drop = start - end;
    (drop.is_finite() && drop > 0.0).then_some(drop * 3_600.0 / seconds)
}

#[cfg(test)]
mod tests {
    use super::{AnomalyKind, InsufficiencyReason, analyze};
    use crate::storage::{
        HistoryAvailability, HistoryFreshness, HistoryMetric, HistoryMetrics, HistorySample,
        HistoryTimelineItem, MetricSource, SampleState,
    };

    fn metric(value: Option<f64>) -> HistoryMetric {
        HistoryMetric {
            value,
            source: value.map_or(MetricSource::Unavailable, |_| MetricSource::Sysfs),
            availability: value.map_or(HistoryAvailability::Unavailable, |_| {
                HistoryAvailability::Available
            }),
            freshness: value.map_or(HistoryFreshness::Unavailable, |_| {
                HistoryFreshness::Recorded
            }),
        }
    }

    fn sample(
        minutes: i64,
        state: SampleState,
        percentage: Option<f64>,
        power: Option<f64>,
    ) -> HistoryTimelineItem {
        HistoryTimelineItem::Sample(Box::new(HistorySample {
            battery_id: "BAT0".to_owned(),
            recorded_at: format!("2026-08-24T00:{minutes:02}:00Z"),
            boot_id: "boot".to_owned(),
            boot_seconds: f64::from(i32::try_from(minutes * 60).expect("test offset fits in i32")),
            state,
            metrics: HistoryMetrics {
                percentage: metric(percentage),
                energy_now_wh: metric(None),
                energy_full_wh: metric(None),
                energy_design_wh: metric(None),
                power_watts: metric(power),
                voltage_volts: metric(None),
                current_amps: metric(None),
                temperature_celsius: metric(None),
                time_remaining_minutes: metric(None),
                cycle_count: metric(None),
            },
        }))
    }

    #[test]
    fn insufficient_history_has_no_fake_alerts() {
        let history = vec![sample(0, SampleState::Discharging, Some(80.0), Some(-8.0))];
        let report = analyze(&history);
        assert_eq!(report.availability, "insufficient");
        assert_eq!(
            report.insufficiency_reason,
            Some(InsufficiencyReason::TooFewSamples)
        );
        assert!(report.anomalies.is_empty());
    }

    #[test]
    fn unusual_power_comes_from_an_observed_outlier() {
        let mut history = Vec::new();
        for minute in 0_i32..5 {
            history.push(sample(
                i64::from(minute),
                SampleState::Discharging,
                Some(80.0 - f64::from(minute)),
                Some(-8.0),
            ));
        }
        history.push(sample(5, SampleState::Discharging, Some(74.0), Some(-30.0)));
        let report = analyze(&history);
        assert_eq!(report.availability, "available");
        assert!(
            report
                .anomalies
                .iter()
                .any(|anomaly| anomaly.kind == AnomalyKind::UnusualPower)
        );
    }

    #[test]
    fn a_gap_prevents_a_synthetic_discharge_drop() {
        let history = vec![
            sample(0, SampleState::Discharging, Some(80.0), Some(-8.0)),
            HistoryTimelineItem::Gap(crate::storage::HistoryGap {
                from: "2026-08-24T00:00:00Z".to_owned(),
                to: "2026-08-24T00:10:00Z".to_owned(),
                reason: crate::storage::HistoryGapReason::SampleIntervalExceeded,
            }),
            sample(10, SampleState::Discharging, Some(10.0), Some(-8.0)),
        ];
        let report = analyze(&history);
        assert_eq!(report.discharge_intervals, 0);
        assert!(report.anomalies.is_empty());
    }
}
