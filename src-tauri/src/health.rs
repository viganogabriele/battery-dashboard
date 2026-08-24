//! Conservative, read-only battery health analysis.
//!
//! This module consumes immutable recorder samples.  It does not interpolate
//! missing telemetry, write derived values back to storage, or treat absent
//! fields as zero.  Capacity values from different batteries are never mixed.

use std::collections::{BTreeMap, BTreeSet};

use time::{Date, OffsetDateTime, format_description::well_known::Rfc3339};

use crate::storage::{HistoryAvailability, HistoryMetric, HistorySample};

const MIN_DAILY_POINTS: usize = 7;
const MIN_TREND_SPAN_DAYS: f64 = 14.0;
const MIN_MEANINGFUL_LOSS_WH_PER_DAY: f64 = 0.01;
const CONFIDENCE_MULTIPLIER: f64 = 2.5;

/// A capacity observation retained with its original sample timestamp.
#[derive(Clone, Debug, PartialEq)]
pub struct CapacityObservation {
    /// The immutable recorder timestamp from which the capacity was read.
    pub recorded_at: String,
    /// Observed full-charge capacity in Wh.
    pub full_capacity_wh: f64,
}

/// An observed capacity, including where in the immutable history it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct CurrentCapacity {
    /// The immutable recorder timestamp from which the capacity was read.
    pub recorded_at: String,
    /// Observed capacity in Wh.
    pub watt_hours: f64,
}

/// A health percentage that can only be produced from a compatible sample pair.
#[derive(Clone, Debug, PartialEq)]
pub struct HealthPercentage {
    /// Timestamp of the single sample that supplied both capacities.
    pub recorded_at: String,
    /// Full-charge capacity divided by design capacity, expressed as a percentage.
    pub percent: f64,
    /// Full-charge capacity used in the ratio.
    pub full_capacity_wh: f64,
    /// Design capacity used in the ratio.
    pub design_capacity_wh: f64,
}

/// The hardware-provided cycle count, not an estimate from charge movement.
#[derive(Clone, Debug, PartialEq)]
pub struct HardwareCycleCount {
    /// Timestamp of the immutable sample carrying the count.
    pub recorded_at: String,
    /// Count reported by the battery hardware/provider.
    pub count: u64,
}

/// Why a daily trend cannot safely make a directional claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrendInsufficiency {
    /// Fewer than the minimum number of valid daily capacity observations existed.
    TooFewDailyObservations,
    /// The daily observations did not cover enough elapsed time.
    TooShortTimeSpan,
}

/// Conservative interpretation of capacity change over time.
#[derive(Clone, Debug, PartialEq)]
pub enum DailyDegradationTrend {
    /// More samples or a longer time window is required.
    Insufficient {
        /// The minimum observation or duration requirement that was not met.
        reason: TrendInsufficiency,
    },
    /// Observations exist, but their noise leaves direction unresolved.
    Inconclusive {
        /// Least-squares capacity change in Wh/day.
        slope_wh_per_day: f64,
        /// Conservative upper confidence bound for the slope in Wh/day.
        upper_confidence_wh_per_day: f64,
    },
    /// The data is compatible with no material daily capacity loss.
    Stable {
        /// Least-squares capacity change in Wh/day.
        slope_wh_per_day: f64,
    },
    /// Capacity loss remains negative after allowing for observed daily noise.
    Degrading {
        /// Least-squares capacity change in Wh/day.
        slope_wh_per_day: f64,
        /// Conservative upper confidence bound for the slope in Wh/day.
        upper_confidence_wh_per_day: f64,
    },
}

/// Complete read-only health result for one physical battery history.
#[derive(Clone, Debug, PartialEq)]
pub struct BatteryHealthReport {
    /// Battery identifier represented by the supplied history, when unambiguous.
    pub battery_id: Option<String>,
    /// Most recent valid full capacity.  This can exist without a design value.
    pub current_full_capacity: Option<CurrentCapacity>,
    /// Most recent valid design capacity.  This can exist without a full value.
    pub current_design_capacity: Option<CurrentCapacity>,
    /// Most recent health ratio whose numerator and denominator were observed together.
    pub health_percentage: Option<HealthPercentage>,
    /// Latest hardware-reported cycle count; this is never inferred.
    pub hardware_cycle_count: Option<HardwareCycleCount>,
    /// Valid full-capacity readings in timestamp order for plotting capacity over time.
    pub capacity_over_time: Vec<CapacityObservation>,
    /// Conservative daily interpretation of the capacity history.
    pub daily_degradation_trend: DailyDegradationTrend,
}

/// Analyses immutable samples for exactly one physical battery.
///
/// If samples contain multiple battery IDs, no capacity values are combined and
/// the report is explicitly insufficient.  Callers should query per battery.
#[must_use]
pub fn analyze(samples: &[HistorySample]) -> BatteryHealthReport {
    let battery_id = samples.first().map(|sample| sample.battery_id.clone());
    let battery_ids = samples
        .iter()
        .map(|sample| sample.battery_id.as_str())
        .collect::<BTreeSet<_>>();
    if battery_ids.len() != 1 {
        // A mixed report must not imply that the first identifier won.  The
        // caller can display a battery selector and query one physical pack.
        return empty_report(None);
    }

    let mut ordered = samples.iter().collect::<Vec<_>>();
    ordered.sort_unstable_by(|left, right| left.recorded_at.cmp(&right.recorded_at));
    let current_full_capacity = latest_metric(&ordered, |sample| sample.metrics.energy_full_wh);
    let current_design_capacity = latest_metric(&ordered, |sample| sample.metrics.energy_design_wh);
    let health_percentage = ordered.iter().rev().find_map(|sample| {
        let full = observed_positive(sample.metrics.energy_full_wh)?;
        let design = observed_positive(sample.metrics.energy_design_wh)?;
        Some(HealthPercentage {
            recorded_at: sample.recorded_at.clone(),
            percent: full / design * 100.0,
            full_capacity_wh: full,
            design_capacity_wh: design,
        })
    });
    let hardware_cycle_count = ordered.iter().rev().find_map(|sample| {
        let metric = sample.metrics.cycle_count;
        if !matches!(
            metric.source,
            crate::storage::MetricSource::Upower | crate::storage::MetricSource::Sysfs
        ) {
            return None;
        }
        let value = observed_non_negative(metric)?;
        let count = (value.fract() == 0.0)
            .then(|| value.to_string().parse::<u64>().ok())
            .flatten()?;
        Some(HardwareCycleCount {
            recorded_at: sample.recorded_at.clone(),
            count,
        })
    });
    let capacity_over_time = ordered
        .iter()
        .filter_map(|sample| {
            observed_positive(sample.metrics.energy_full_wh).map(|full_capacity_wh| {
                CapacityObservation {
                    recorded_at: sample.recorded_at.clone(),
                    full_capacity_wh,
                }
            })
        })
        .collect::<Vec<_>>();
    let daily_degradation_trend = trend(&capacity_over_time);

    BatteryHealthReport {
        battery_id,
        current_full_capacity,
        current_design_capacity,
        health_percentage,
        hardware_cycle_count,
        capacity_over_time,
        daily_degradation_trend,
    }
}

fn empty_report(battery_id: Option<String>) -> BatteryHealthReport {
    BatteryHealthReport {
        battery_id,
        current_full_capacity: None,
        current_design_capacity: None,
        health_percentage: None,
        hardware_cycle_count: None,
        capacity_over_time: Vec::new(),
        daily_degradation_trend: DailyDegradationTrend::Insufficient {
            reason: TrendInsufficiency::TooFewDailyObservations,
        },
    }
}

fn latest_metric(
    samples: &[&HistorySample],
    select: impl Fn(&HistorySample) -> HistoryMetric,
) -> Option<CurrentCapacity> {
    samples.iter().rev().find_map(|sample| {
        observed_positive(select(sample)).map(|watt_hours| CurrentCapacity {
            recorded_at: sample.recorded_at.clone(),
            watt_hours,
        })
    })
}

fn observed_positive(metric: HistoryMetric) -> Option<f64> {
    (metric.availability == HistoryAvailability::Available)
        .then_some(metric.value)
        .flatten()
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn observed_non_negative(metric: HistoryMetric) -> Option<f64> {
    (metric.availability == HistoryAvailability::Available)
        .then_some(metric.value)
        .flatten()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn trend(observations: &[CapacityObservation]) -> DailyDegradationTrend {
    let mut daily = BTreeMap::<Date, Vec<f64>>::new();
    for observation in observations {
        let Ok(timestamp) = OffsetDateTime::parse(&observation.recorded_at, &Rfc3339) else {
            continue;
        };
        daily
            .entry(timestamp.date())
            .or_default()
            .push(observation.full_capacity_wh);
    }
    if daily.len() < MIN_DAILY_POINTS {
        return DailyDegradationTrend::Insufficient {
            reason: TrendInsufficiency::TooFewDailyObservations,
        };
    }
    let first = *daily
        .first_key_value()
        .expect("non-empty after length check")
        .0;
    let last = *daily
        .last_key_value()
        .expect("non-empty after length check")
        .0;
    let span_days = days_as_f64((last - first).whole_days());
    if span_days < MIN_TREND_SPAN_DAYS {
        return DailyDegradationTrend::Insufficient {
            reason: TrendInsufficiency::TooShortTimeSpan,
        };
    }
    let points = daily
        .into_iter()
        .map(|(date, values)| {
            let maximum = values
                .into_iter()
                .max_by(f64::total_cmp)
                .expect("daily bucket is non-empty");
            (days_as_f64((date - first).whole_days()), maximum)
        })
        .collect::<Vec<_>>();
    let (slope, upper) = regression_with_upper_bound(&points);
    if upper < -MIN_MEANINGFUL_LOSS_WH_PER_DAY {
        DailyDegradationTrend::Degrading {
            slope_wh_per_day: slope,
            upper_confidence_wh_per_day: upper,
        }
    } else if slope.abs() <= MIN_MEANINGFUL_LOSS_WH_PER_DAY {
        DailyDegradationTrend::Stable {
            slope_wh_per_day: slope,
        }
    } else {
        DailyDegradationTrend::Inconclusive {
            slope_wh_per_day: slope,
            upper_confidence_wh_per_day: upper,
        }
    }
}

fn days_as_f64(days: i64) -> f64 {
    // `time::Date`'s supported range is much narrower than `i32`.
    f64::from(i32::try_from(days).expect("date duration fits in i32 days"))
}

fn regression_with_upper_bound(points: &[(f64, f64)]) -> (f64, f64) {
    let count = f64::from(u32::try_from(points.len()).expect("point count fits in u32"));
    let mean_x = points.iter().map(|(x, _)| x).sum::<f64>() / count;
    let mean_y = points.iter().map(|(_, y)| y).sum::<f64>() / count;
    let centered_xx = points
        .iter()
        .map(|(x, _)| (x - mean_x).powi(2))
        .sum::<f64>();
    let slope = points
        .iter()
        .map(|(x, y)| (x - mean_x) * (y - mean_y))
        .sum::<f64>()
        / centered_xx;
    let intercept = mean_y - slope * mean_x;
    let residual_sum = points
        .iter()
        .map(|(x, y)| (y - (intercept + slope * x)).powi(2))
        .sum::<f64>();
    let standard_error = (residual_sum / (count - 2.0) / centered_xx).sqrt();
    (slope, slope + CONFIDENCE_MULTIPLIER * standard_error)
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
        day: i64,
        full: Option<f64>,
        design: Option<f64>,
        cycles: Option<f64>,
    ) -> HistorySample {
        let timestamp = OffsetDateTime::from_unix_timestamp(1_704_067_200 + day * 86_400).unwrap();
        HistorySample {
            battery_id: "BAT0".into(),
            recorded_at: timestamp.format(&Rfc3339).unwrap(),
            boot_id: "boot".into(),
            boot_seconds: 0.0,
            state: SampleState::Discharging,
            metrics: HistoryMetrics {
                percentage: metric(None),
                energy_now_wh: metric(None),
                energy_full_wh: metric(full),
                energy_design_wh: metric(design),
                power_watts: metric(None),
                voltage_volts: metric(None),
                current_amps: metric(None),
                temperature_celsius: metric(None),
                time_remaining_minutes: metric(None),
                cycle_count: metric(cycles),
            },
        }
    }

    #[test]
    fn stable_history_is_not_called_degrading() {
        let history = (0..21)
            .map(|day| {
                sample(
                    day,
                    Some(50.0 + if day % 2 == 0 { 0.02 } else { -0.02 }),
                    Some(60.0),
                    Some(100.0),
                )
            })
            .collect::<Vec<_>>();
        let report = analyze(&history);
        assert!(matches!(
            report.daily_degradation_trend,
            DailyDegradationTrend::Stable { .. }
        ));
        assert!((report.health_percentage.unwrap().percent - 83.37).abs() < 0.01);
        assert_eq!(report.hardware_cycle_count.unwrap().count, 100);
    }

    #[test]
    fn sustained_loss_is_reported_as_degrading() {
        let history = (0..30)
            .map(|day| {
                sample(
                    day,
                    Some(55.0 - f64::from(i32::try_from(day).unwrap()) * 0.12),
                    Some(60.0),
                    None,
                )
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            analyze(&history).daily_degradation_trend,
            DailyDegradationTrend::Degrading { .. }
        ));
    }

    #[test]
    fn noisy_history_is_inconclusive_not_overclaimed() {
        let noise = [0.8, -0.7, 0.6, -0.9, 0.7, -0.6, 0.9];
        let history = (0..28)
            .map(|day| {
                sample(
                    day,
                    Some(
                        50.0 - f64::from(i32::try_from(day).unwrap()) * 0.04
                            + noise[usize::try_from(day % 7).unwrap()],
                    ),
                    Some(60.0),
                    None,
                )
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            analyze(&history).daily_degradation_trend,
            DailyDegradationTrend::Inconclusive { .. }
        ));
    }

    #[test]
    fn missing_values_do_not_become_zero_or_health() {
        let history = vec![
            sample(0, Some(50.0), None, Some(12.5)),
            sample(1, None, Some(60.0), Some(13.0)),
        ];
        let report = analyze(&history);
        assert!((report.current_full_capacity.unwrap().watt_hours - 50.0).abs() < f64::EPSILON);
        assert!((report.current_design_capacity.unwrap().watt_hours - 60.0).abs() < f64::EPSILON);
        assert!(report.health_percentage.is_none());
        assert_eq!(report.hardware_cycle_count.unwrap().count, 13);
        assert!(matches!(
            report.daily_degradation_trend,
            DailyDegradationTrend::Insufficient { .. }
        ));
    }

    #[test]
    fn mixed_batteries_are_not_reported_as_the_first_battery() {
        let first = sample(0, Some(50.0), Some(60.0), Some(12.0));
        let mut second = sample(1, Some(40.0), Some(50.0), Some(8.0));
        second.battery_id = "BAT1".to_owned();
        let report = analyze(&[first, second]);
        assert_eq!(report.battery_id, None);
        assert!(report.current_full_capacity.is_none());
        assert!(report.health_percentage.is_none());
    }

    #[test]
    fn derived_cycle_count_is_not_presented_as_hardware_count() {
        let mut sample = sample(0, Some(50.0), Some(60.0), Some(12.0));
        sample.metrics.cycle_count.source = MetricSource::Derived;
        assert!(analyze(&[sample]).hardware_cycle_count.is_none());
    }
}
