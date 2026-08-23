import type {
  BatteryState,
  Metric,
  MetricAvailability,
  MetricSource,
} from '../domain/battery';

/** The fixed windows supported by the first recent-history dashboard. */
export type RecentHistoryRangeHours = 2 | 6 | 12 | 24;

/** Request passed to the `get_recent_battery_history` desktop command. */
export interface RecentBatteryHistoryRequest {
  /** Omit this to ask for the command's all-battery view. */
  batteryId?: string;
  rangeHours: RecentHistoryRangeHours;
  /** Upper bound for returned points after the provider has reduced the series. */
  maxPoints: number;
}

export type RecentHistoryAvailability = 'available' | 'unavailable';
export type RecentHistorySource = 'sqlite' | 'transient' | 'unavailable';
export type HistoryFreshness = 'fresh' | 'stale' | 'unknown';
export type HistoryPointKind = 'persisted' | 'transient';

/** Why an otherwise requested portion of the chart cannot be represented. */
export type HistoryGapReason =
  | 'recorder-disabled'
  | 'suspended'
  | 'rebooted'
  | 'missing-samples'
  | 'database-unavailable'
  | 'unknown';

export type RecentHistoryUnavailableReason =
  | 'unsupported'
  | 'recorder-disabled'
  | 'no-recorded-samples'
  | 'database-unavailable'
  | 'invalid-request'
  | 'unknown';

/** Raw metric format shared by persisted and intentionally transient points. */
export interface HistoryMetricResponseDto {
  value: number | null;
  source: Exclude<MetricSource, 'simulated'>;
  availability: MetricAvailability;
  observedAt: string | null;
}

export interface BatteryHistoryMetricsResponseDto {
  percentage: HistoryMetricResponseDto;
  energyNowWh: HistoryMetricResponseDto;
  powerWatts: HistoryMetricResponseDto;
}

/**
 * A single point in chronological history. A `transient` point is explicitly
 * labelled so the UI never presents an unrecorded current reading as SQLite
 * history.
 */
export interface BatteryHistoryPointResponseDto {
  batteryId: string;
  recordedAt: string;
  kind: HistoryPointKind;
  state: BatteryState;
  freshness: HistoryFreshness;
  metrics: BatteryHistoryMetricsResponseDto;
}

/** An intentional discontinuity rather than an interpolated chart segment. */
export interface BatteryHistoryGapResponseDto {
  startsAt: string;
  endsAt: string | null;
  reason: HistoryGapReason;
  detail: string | null;
}

/**
 * The server calculates summaries only from observed values. A null value is
 * not a zero and must remain unavailable to the presentation layer.
 */
export interface NumericHistorySummaryResponseDto {
  minimum: number | null;
  maximum: number | null;
  average: number | null;
  observedSamples: number;
  source: 'derived' | 'unavailable';
  availability: MetricAvailability;
  observedAt: string | null;
}

/** The first/last recorded energy and their difference, never an estimate. */
export interface ObservedEnergySummaryResponseDto {
  first: number | null;
  last: number | null;
  change: number | null;
  observedSamples: number;
  source: 'derived' | 'unavailable';
  availability: MetricAvailability;
  observedAt: string | null;
}

export interface RecentBatteryHistorySummaryResponseDto {
  percentage: NumericHistorySummaryResponseDto;
  powerWatts: NumericHistorySummaryResponseDto;
  energyNowWh: NumericHistorySummaryResponseDto;
  observedEnergyWh: ObservedEnergySummaryResponseDto;
}

/** Exact camelCase payload returned by `get_recent_battery_history`. */
export interface RecentBatteryHistoryResponseDto {
  schemaVersion: 1;
  availability: RecentHistoryAvailability;
  unavailableReason: RecentHistoryUnavailableReason | null;
  source: RecentHistorySource;
  freshness: HistoryFreshness;
  batteryId: string | null;
  rangeHours: RecentHistoryRangeHours;
  collectedAt: string | null;
  points: readonly BatteryHistoryPointResponseDto[];
  gaps: readonly BatteryHistoryGapResponseDto[];
  summary: RecentBatteryHistorySummaryResponseDto;
}

/** Framework-neutral form consumed by chart and summary components. */
export interface BatteryHistoryPoint {
  batteryId: string;
  recordedAt: string;
  kind: HistoryPointKind;
  state: BatteryState;
  freshness: HistoryFreshness;
  percentage: Metric<number>;
  energyNowWh: Metric<number>;
  powerWatts: Metric<number>;
}

export interface NumericHistorySummary {
  minimum: number | null;
  maximum: number | null;
  average: number | null;
  observedSamples: number;
  source: 'derived' | 'unavailable';
  availability: MetricAvailability;
  observedAt: string | null;
}

export interface ObservedEnergySummary {
  first: number | null;
  last: number | null;
  change: number | null;
  observedSamples: number;
  source: 'derived' | 'unavailable';
  availability: MetricAvailability;
  observedAt: string | null;
}

export interface RecentBatteryHistoryData {
  availability: RecentHistoryAvailability;
  unavailableReason: RecentHistoryUnavailableReason | null;
  source: RecentHistorySource;
  freshness: HistoryFreshness;
  batteryId: string | null;
  rangeHours: RecentHistoryRangeHours;
  collectedAt: string | null;
  points: readonly BatteryHistoryPoint[];
  gaps: readonly BatteryHistoryGapResponseDto[];
  summary: {
    percentage: NumericHistorySummary;
    powerWatts: NumericHistorySummary;
    energyNowWh: NumericHistorySummary;
    observedEnergyWh: ObservedEnergySummary;
  };
}

/** Tauri-independent boundary used by Svelte and deterministic tests. */
export interface RecentBatteryHistoryClient {
  getRecentHistory(
    request: RecentBatteryHistoryRequest,
  ): Promise<RecentBatteryHistoryData>;
}

export interface RecentHistoryCommandInvoker {
  <Response>(
    command: 'get_recent_battery_history',
    arguments_: {
      batteryId?: string;
      rangeHours: RecentHistoryRangeHours;
      maxPoints: number;
    },
  ): Promise<Response>;
}

export function createRecentBatteryHistoryClient(
  invoke: RecentHistoryCommandInvoker,
): RecentBatteryHistoryClient {
  return {
    async getRecentHistory(request) {
      return normalizeRecentBatteryHistoryResponse(
        await invoke<RecentBatteryHistoryResponseDto>(
          'get_recent_battery_history',
          toCommandArguments(request),
        ),
      );
    },
  };
}

/**
 * Browser preview intentionally reports unsupported history. It does not
 * return fixture points, which would make an opt-in recorded-history view lie.
 */
export function createDesktopRecentBatteryHistoryClient(): RecentBatteryHistoryClient {
  if (!isTauriRuntime()) return createUnsupportedRecentBatteryHistoryClient();

  return {
    async getRecentHistory(request) {
      const { invoke } = await import('@tauri-apps/api/core');
      return normalizeRecentBatteryHistoryResponse(
        await invoke<RecentBatteryHistoryResponseDto>(
          'get_recent_battery_history',
          toCommandArguments(request),
        ),
      );
    },
  };
}

export function createUnsupportedRecentBatteryHistoryClient(): RecentBatteryHistoryClient {
  return {
    async getRecentHistory(request) {
      return unsupportedHistory(request);
    },
  };
}

export function normalizeRecentBatteryHistoryResponse(
  response: RecentBatteryHistoryResponseDto,
): RecentBatteryHistoryData {
  return {
    availability: response.availability,
    unavailableReason: response.unavailableReason,
    source: response.source,
    freshness: response.freshness,
    batteryId: response.batteryId,
    rangeHours: response.rangeHours,
    collectedAt: response.collectedAt,
    points: response.points.map(normalizePoint),
    gaps: response.gaps.map((gap) => ({ ...gap })),
    summary: {
      percentage: normalizeNumericSummary(response.summary.percentage),
      powerWatts: normalizeNumericSummary(response.summary.powerWatts),
      energyNowWh: normalizeNumericSummary(response.summary.energyNowWh),
      observedEnergyWh: normalizeObservedEnergy(response.summary.observedEnergyWh),
    },
  };
}

function toCommandArguments(request: RecentBatteryHistoryRequest) {
  return request.batteryId === undefined
    ? { rangeHours: request.rangeHours, maxPoints: request.maxPoints }
    : {
        batteryId: request.batteryId,
        rangeHours: request.rangeHours,
        maxPoints: request.maxPoints,
      };
}

function normalizePoint(response: BatteryHistoryPointResponseDto): BatteryHistoryPoint {
  return {
    batteryId: response.batteryId,
    recordedAt: response.recordedAt,
    kind: response.kind,
    state: response.state,
    freshness: response.freshness,
    percentage: normalizeMetric(response.metrics.percentage),
    energyNowWh: normalizeMetric(response.metrics.energyNowWh),
    powerWatts: normalizeMetric(response.metrics.powerWatts),
  };
}

function normalizeMetric(response: HistoryMetricResponseDto): Metric<number> {
  if (response.value === null || response.availability === 'unavailable') {
    return {
      value: null,
      source: 'unavailable',
      availability: 'unavailable',
      updatedAt: null,
    };
  }

  return {
    value: response.value,
    source: response.source,
    availability: response.availability,
    updatedAt: response.observedAt,
  };
}

function normalizeNumericSummary(
  response: NumericHistorySummaryResponseDto,
): NumericHistorySummary {
  if (
    response.availability === 'unavailable' ||
    response.minimum === null ||
    response.maximum === null ||
    response.average === null
  ) {
    return unavailableNumericSummary(response.observedSamples);
  }

  return { ...response };
}

function normalizeObservedEnergy(
  response: ObservedEnergySummaryResponseDto,
): ObservedEnergySummary {
  if (
    response.availability === 'unavailable' ||
    response.first === null ||
    response.last === null ||
    response.change === null
  ) {
    return unavailableObservedEnergy(response.observedSamples);
  }

  return { ...response };
}

function unsupportedHistory(
  request: RecentBatteryHistoryRequest,
): RecentBatteryHistoryData {
  return {
    availability: 'unavailable',
    unavailableReason: 'unsupported',
    source: 'unavailable',
    freshness: 'unknown',
    batteryId: request.batteryId ?? null,
    rangeHours: request.rangeHours,
    collectedAt: null,
    points: [],
    gaps: [],
    summary: unavailableSummary(),
  };
}

function unavailableSummary(): RecentBatteryHistoryData['summary'] {
  return {
    percentage: unavailableNumericSummary(0),
    powerWatts: unavailableNumericSummary(0),
    energyNowWh: unavailableNumericSummary(0),
    observedEnergyWh: unavailableObservedEnergy(0),
  };
}

function unavailableNumericSummary(observedSamples: number): NumericHistorySummary {
  return {
    minimum: null,
    maximum: null,
    average: null,
    observedSamples,
    source: 'unavailable',
    availability: 'unavailable',
    observedAt: null,
  };
}

function unavailableObservedEnergy(observedSamples: number): ObservedEnergySummary {
  return {
    first: null,
    last: null,
    change: null,
    observedSamples,
    source: 'unavailable',
    availability: 'unavailable',
    observedAt: null,
  };
}

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
