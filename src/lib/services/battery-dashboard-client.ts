import {
  aggregateBatteries,
  type AggregateBatterySnapshot,
  type BatterySnapshot,
  type BatteryState,
  type Metric,
  type MetricAvailability,
  type MetricSource,
} from '../domain/battery';

/**
 * Exact payload returned by the future `get_battery_dashboard` Tauri command.
 *
 * Rust should serialize these names in camelCase. Numeric readings use `null`
 * when a driver or provider does not expose them; they must never be replaced
 * with zero. `stale` is set after resume or when the last successful reading is
 * older than the recorder's freshness threshold.
 */
export interface BatteryDashboardResponseDto {
  schemaVersion: 1;
  collectedAt: string | null;
  stale: boolean;
  batteries: readonly BatteryResponseDto[];
}

export interface BatteryResponseDto {
  id: string;
  label: string;
  state: BatteryState;
  updatedAt: string | null;
  metrics: BatteryMetricsResponseDto;
}

export interface BatteryMetricsResponseDto {
  percentage: BatteryMetricResponseDto;
  energyNowWh: BatteryMetricResponseDto;
  energyFullWh: BatteryMetricResponseDto;
  energyDesignWh: BatteryMetricResponseDto;
  powerWatts: BatteryMetricResponseDto;
  voltageVolts: BatteryMetricResponseDto;
  currentAmps: BatteryMetricResponseDto;
  temperatureCelsius: BatteryMetricResponseDto;
  timeRemainingMinutes: BatteryMetricResponseDto;
  cycleCount: BatteryMetricResponseDto;
}

export interface BatteryMetricResponseDto {
  value: number | null;
  source: Exclude<MetricSource, 'simulated'>;
  availability: MetricAvailability;
  updatedAt: string | null;
}

/** Framework-neutral shape consumed by the Svelte dashboard. */
export interface BatteryDashboardData {
  collectedAt: string | null;
  stale: boolean;
  batteries: readonly BatterySnapshot[];
  aggregate: AggregateBatterySnapshot;
  selectedSnapshot: BatterySnapshot | AggregateBatterySnapshot | null;
}

/**
 * Boundary for the desktop shell. The eventual Tauri adapter only needs to
 * provide this function with `invoke<BatteryDashboardResponseDto>(...)`.
 */
export interface BatteryDashboardClient {
  getDashboard(): Promise<BatteryDashboardData>;
}

export type BatteryDashboardLoader = () => Promise<BatteryDashboardResponseDto>;

export function createBatteryDashboardClient(
  loadResponse: BatteryDashboardLoader,
): BatteryDashboardClient {
  return {
    async getDashboard() {
      return normalizeBatteryDashboardResponse(await loadResponse());
    },
  };
}

/** A deterministic client useful for UI previews and tests without Tauri. */
export function createFixtureBatteryDashboardClient(
  response: BatteryDashboardResponseDto,
): BatteryDashboardClient {
  return createBatteryDashboardClient(async () => response);
}

export function normalizeBatteryDashboardResponse(
  response: BatteryDashboardResponseDto,
): BatteryDashboardData {
  const batteries = response.batteries.map((battery) =>
    normalizeBattery(battery, response.stale),
  );
  const aggregate = aggregateBatteries(batteries);

  return {
    collectedAt: response.collectedAt,
    stale: response.stale,
    batteries,
    aggregate,
    selectedSnapshot: batteries.length === 0 ? null : aggregate,
  };
}

function normalizeBattery(
  response: BatteryResponseDto,
  dashboardIsStale: boolean,
): BatterySnapshot {
  return {
    kind: 'battery',
    id: response.id,
    label: response.label,
    state: response.state,
    percentage: normalizeMetric(response.metrics.percentage, dashboardIsStale),
    energyNowWh: normalizeMetric(response.metrics.energyNowWh, dashboardIsStale),
    energyFullWh: normalizeMetric(response.metrics.energyFullWh, dashboardIsStale),
    energyDesignWh: normalizeMetric(response.metrics.energyDesignWh, dashboardIsStale),
    powerWatts: normalizeMetric(response.metrics.powerWatts, dashboardIsStale),
    voltageVolts: normalizeMetric(response.metrics.voltageVolts, dashboardIsStale),
    currentAmps: normalizeMetric(response.metrics.currentAmps, dashboardIsStale),
    temperatureCelsius: normalizeMetric(
      response.metrics.temperatureCelsius,
      dashboardIsStale,
    ),
    timeRemainingMinutes: normalizeMetric(
      response.metrics.timeRemainingMinutes,
      dashboardIsStale,
    ),
    cycleCount: normalizeMetric(response.metrics.cycleCount, dashboardIsStale),
    updatedAt: response.updatedAt,
  };
}

function normalizeMetric(
  metric: BatteryMetricResponseDto,
  dashboardIsStale: boolean,
): Metric<number> {
  if (metric.value === null || metric.availability === 'unavailable') {
    return {
      value: null,
      source: 'unavailable',
      availability: 'unavailable',
      updatedAt: null,
    };
  }

  return {
    value: metric.value,
    source: metric.source,
    availability: dashboardIsStale ? 'stale' : metric.availability,
    updatedAt: metric.updatedAt,
  };
}
