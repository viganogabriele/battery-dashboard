/**
 * Frontend-only battery contracts. Real providers will map UPower and sysfs
 * values into these shapes in a later phase.
 */
export type MetricSource = 'simulated' | 'upower' | 'sysfs' | 'derived' | 'unavailable';

export type MetricAvailability = 'available' | 'unavailable' | 'stale';

export interface Metric<T> {
  value: T | null;
  source: MetricSource;
  availability: MetricAvailability;
  updatedAt: string | null;
}

export type BatteryState = 'charging' | 'discharging' | 'full' | 'idle' | 'unknown';

export type AggregateBatteryState = BatteryState | 'mixed';

export interface BatterySnapshot {
  kind: 'battery';
  id: string;
  label: string;
  state: BatteryState;
  percentage: Metric<number>;
  energyNowWh: Metric<number>;
  energyFullWh: Metric<number>;
  energyDesignWh: Metric<number>;
  powerWatts: Metric<number>;
  voltageVolts: Metric<number>;
  currentAmps: Metric<number>;
  temperatureCelsius: Metric<number>;
  timeRemainingMinutes: Metric<number>;
  cycleCount: Metric<number>;
  updatedAt: string | null;
}

export interface AggregateBatterySnapshot {
  kind: 'aggregate';
  id: 'all-batteries';
  label: string;
  state: AggregateBatteryState;
  batteryCount: number;
  percentage: Metric<number>;
  energyNowWh: Metric<number>;
  energyFullWh: Metric<number>;
  energyDesignWh: Metric<number>;
  powerWatts: Metric<number>;
  /** Only meaningful for a single selected battery. */
  voltageVolts: Metric<number>;
  /** Only meaningful for a single selected battery. */
  currentAmps: Metric<number>;
  /** Only meaningful for a single selected battery. */
  temperatureCelsius: Metric<number>;
  timeRemainingMinutes: Metric<number>;
  cycleCount: Metric<number>;
  updatedAt: string | null;
}

export type DashboardSnapshot = BatterySnapshot | AggregateBatterySnapshot;

export interface BatteryChartPoint {
  timestamp: string;
  percentage: number | null;
  powerWatts: number | null;
  state: BatteryState;
}

export interface DashboardScenario {
  id: string;
  name: string;
  description: string;
  batteries: readonly BatterySnapshot[];
  aggregate: AggregateBatterySnapshot;
  selectedSnapshot: DashboardSnapshot | null;
  chart: readonly BatteryChartPoint[];
}

export interface DashboardScenarioCatalog {
  defaultScenarioId: string;
  scenarios: readonly DashboardScenario[];
}

export function availableMetric<T>(
  value: T,
  updatedAt: string,
  source: MetricSource = 'simulated',
): Metric<T> {
  return { value, source, availability: 'available', updatedAt };
}

export function unavailableMetric<T>(): Metric<T> {
  return {
    value: null,
    source: 'unavailable',
    availability: 'unavailable',
    updatedAt: null,
  };
}

export function staleMetric<T>(
  value: T,
  updatedAt: string,
  source: MetricSource = 'simulated',
): Metric<T> {
  return { value, source, availability: 'stale', updatedAt };
}

export function isMetricAvailable<T>(
  metric: Metric<T>,
): metric is Metric<T> & { value: T } {
  return metric.value !== null && metric.availability !== 'unavailable';
}

function unavailableAggregateMetric(): Metric<number> {
  return unavailableMetric<number>();
}

function aggregateNumericMetric(
  batteries: readonly BatterySnapshot[],
  getMetric: (battery: BatterySnapshot) => Metric<number>,
): Metric<number> {
  if (batteries.length === 0) return unavailableAggregateMetric();

  const metrics = batteries.map(getMetric);
  if (!metrics.every(isMetricAvailable)) return unavailableAggregateMetric();

  const timestamps = metrics
    .map((metric) => metric.updatedAt)
    .filter((timestamp): timestamp is string => timestamp !== null);
  return {
    value: metrics.reduce((sum, metric) => sum + metric.value, 0),
    source: 'derived',
    availability: metrics.some((metric) => metric.availability === 'stale')
      ? 'stale'
      : 'available',
    updatedAt: timestamps.length === 0 ? null : (timestamps.sort().at(-1) ?? null),
  };
}

function aggregatePercentage(batteries: readonly BatterySnapshot[]): Metric<number> {
  if (batteries.length === 0) return unavailableAggregateMetric();

  const parts = batteries.map((battery) => ({
    percentage: battery.percentage,
    capacity: battery.energyFullWh,
  }));
  const availableParts = parts.filter(
    (
      part,
    ): part is {
      percentage: Metric<number> & { value: number };
      capacity: Metric<number> & { value: number };
    } => isMetricAvailable(part.percentage) && isMetricAvailable(part.capacity),
  );
  if (availableParts.length !== batteries.length) {
    return unavailableAggregateMetric();
  }

  const totalCapacity = availableParts.reduce(
    (sum, part) => sum + part.capacity.value,
    0,
  );
  if (totalCapacity <= 0) return unavailableAggregateMetric();

  const latestTimestamp =
    availableParts
      .flatMap(({ percentage, capacity }) => [percentage.updatedAt, capacity.updatedAt])
      .filter((timestamp): timestamp is string => timestamp !== null)
      .sort()
      .at(-1) ?? null;

  return {
    value:
      availableParts.reduce(
        (sum, part) => sum + part.percentage.value * part.capacity.value,
        0,
      ) / totalCapacity,
    source: 'derived',
    availability: availableParts.some(
      ({ percentage, capacity }) =>
        percentage.availability === 'stale' || capacity.availability === 'stale',
    )
      ? 'stale'
      : 'available',
    updatedAt: latestTimestamp,
  };
}

function aggregateState(batteries: readonly BatterySnapshot[]): AggregateBatteryState {
  if (batteries.length === 0) return 'unknown';
  const states = new Set(batteries.map((battery) => battery.state));
  return states.size === 1 ? (batteries[0]?.state ?? 'unknown') : 'mixed';
}

function copySingleMetric(
  batteries: readonly BatterySnapshot[],
  getMetric: (battery: BatterySnapshot) => Metric<number>,
): Metric<number> {
  return batteries.length === 1 && batteries[0]
    ? getMetric(batteries[0])
    : unavailableAggregateMetric();
}

/**
 * Builds an honest "all batteries" view. Values that cannot safely be
 * combined, such as voltage and temperature across several packs, are null.
 */
export function aggregateBatteries(
  batteries: readonly BatterySnapshot[],
): AggregateBatterySnapshot {
  const latestTimestamp =
    batteries
      .map((battery) => battery.updatedAt)
      .filter((timestamp): timestamp is string => timestamp !== null)
      .sort()
      .at(-1) ?? null;

  return {
    kind: 'aggregate',
    id: 'all-batteries',
    label:
      batteries.length === 1
        ? (batteries[0]?.label ?? 'All batteries')
        : 'All batteries',
    state: aggregateState(batteries),
    batteryCount: batteries.length,
    percentage: aggregatePercentage(batteries),
    energyNowWh: aggregateNumericMetric(batteries, (battery) => battery.energyNowWh),
    energyFullWh: aggregateNumericMetric(batteries, (battery) => battery.energyFullWh),
    energyDesignWh: aggregateNumericMetric(
      batteries,
      (battery) => battery.energyDesignWh,
    ),
    powerWatts: aggregateNumericMetric(batteries, (battery) => battery.powerWatts),
    voltageVolts: copySingleMetric(batteries, (battery) => battery.voltageVolts),
    currentAmps: copySingleMetric(batteries, (battery) => battery.currentAmps),
    temperatureCelsius: copySingleMetric(
      batteries,
      (battery) => battery.temperatureCelsius,
    ),
    timeRemainingMinutes: copySingleMetric(
      batteries,
      (battery) => battery.timeRemainingMinutes,
    ),
    cycleCount: copySingleMetric(batteries, (battery) => battery.cycleCount),
    updatedAt: latestTimestamp,
  };
}
