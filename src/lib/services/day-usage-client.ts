/** Today-vs-yesterday observed usage, built entirely from recorded SQLite
 * samples. Every derived figure stays `null` unless the underlying
 * observations directly support it; nothing here is interpolated or
 * estimated across a suspend/reboot/battery-removal gap. */

export type DayUsageAvailability = 'available' | 'unavailable';
export type DayUsageUnavailableReason =
  | 'unsupported'
  | 'recorder-disabled'
  | 'no-recorded-samples'
  | 'database-unavailable'
  | 'invalid-request'
  | 'unknown';

export type DayUsageEvidence = 'sufficient' | 'insufficient';
export type DayUsageInsufficientReason = 'no-recording' | 'too-few-samples';

/** Exact camelCase payload for one local calendar day. */
export interface DayUsageResponseDto {
  available: boolean;
  date: string;
  dayStart: string | null;
  dayEnd: string | null;
  evidence: DayUsageEvidence;
  insufficientReason: DayUsageInsufficientReason | null;
  sampleCount: number;
  elapsedSeconds: number;
  observedDurationSeconds: number | null;
  coverageRatio: number | null;
  startPercentage: number | null;
  endPercentage: number | null;
  percentageChange: number | null;
  energyChangeWh: number | null;
  averageDischargePowerWatts: number | null;
  averageChargePowerWatts: number | null;
  /** Set only for the aggregate "all batteries" scope. */
  contributingBatteries: number | null;
}

export interface TodayVsYesterdayResponseDto {
  schemaVersion: 1;
  availability: DayUsageAvailability;
  unavailableReason: DayUsageUnavailableReason | null;
  generatedAt: string | null;
  timezone: string;
  batteryId: string | null;
  today: DayUsageResponseDto;
  yesterday: DayUsageResponseDto;
}

export interface DayUsageComparisonData {
  availability: DayUsageAvailability;
  unavailableReason: DayUsageUnavailableReason | null;
  generatedAt: string | null;
  timezone: string;
  batteryId: string | null;
  today: DayUsageResponseDto;
  yesterday: DayUsageResponseDto;
}

export interface DayUsageRequest {
  /** Omit for the explicit all-battery view. */
  batteryId?: string;
  /** IANA timezone used to resolve local calendar-day boundaries. */
  timezone: string;
}

export interface DayUsageClient {
  getTodayVsYesterday(request: DayUsageRequest): Promise<DayUsageComparisonData>;
}

export interface DayUsageCommandInvoker {
  <Response>(
    command: 'get_today_vs_yesterday_usage',
    arguments_: { batteryId?: string; timezone: string },
  ): Promise<Response>;
}

export function createDayUsageClient(invoke: DayUsageCommandInvoker): DayUsageClient {
  return {
    async getTodayVsYesterday(request) {
      return normalizeDayUsageResponse(
        await invoke<TodayVsYesterdayResponseDto>(
          'get_today_vs_yesterday_usage',
          toCommandArguments(request),
        ),
      );
    },
  };
}

/** Browser previews must not manufacture a today/yesterday comparison. */
export function createDesktopDayUsageClient(): DayUsageClient {
  if (!isTauriRuntime()) return createUnsupportedDayUsageClient();

  return {
    async getTodayVsYesterday(request) {
      const { invoke } = await import('@tauri-apps/api/core');
      return normalizeDayUsageResponse(
        await invoke<TodayVsYesterdayResponseDto>(
          'get_today_vs_yesterday_usage',
          toCommandArguments(request),
        ),
      );
    },
  };
}

export function createUnsupportedDayUsageClient(): DayUsageClient {
  return {
    async getTodayVsYesterday(request) {
      return unsupportedComparison(request.timezone);
    },
  };
}

export function normalizeDayUsageResponse(
  response: TodayVsYesterdayResponseDto,
): DayUsageComparisonData {
  return {
    availability: response.availability,
    unavailableReason: response.unavailableReason,
    generatedAt: response.generatedAt,
    timezone: response.timezone,
    batteryId: response.batteryId,
    today: { ...response.today },
    yesterday: { ...response.yesterday },
  };
}

function toCommandArguments(request: DayUsageRequest) {
  return {
    ...(request.batteryId === undefined ? {} : { batteryId: request.batteryId }),
    timezone: request.timezone,
  };
}

function unavailableDay(): DayUsageResponseDto {
  return {
    available: false,
    date: '',
    dayStart: null,
    dayEnd: null,
    evidence: 'insufficient',
    insufficientReason: 'no-recording',
    sampleCount: 0,
    elapsedSeconds: 0,
    observedDurationSeconds: null,
    coverageRatio: null,
    startPercentage: null,
    endPercentage: null,
    percentageChange: null,
    energyChangeWh: null,
    averageDischargePowerWatts: null,
    averageChargePowerWatts: null,
    contributingBatteries: null,
  };
}

function unsupportedComparison(timezone: string): DayUsageComparisonData {
  return {
    availability: 'unavailable',
    unavailableReason: 'unsupported',
    generatedAt: null,
    timezone,
    batteryId: null,
    today: unavailableDay(),
    yesterday: unavailableDay(),
  };
}

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
