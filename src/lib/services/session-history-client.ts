import type { BatteryState } from '../domain/battery';

/** Inclusive local-calendar range interpreted in the requested IANA timezone. */
export interface SessionHistoryRequest {
  /** Omit for the explicit all-battery view. */
  batteryId?: string;
  /** Omit to retain every recorded session state. */
  states?: readonly BatteryState[];
  /** ISO local date (`YYYY-MM-DD`), inclusive. */
  startDate?: string;
  /** ISO local date (`YYYY-MM-DD`), inclusive. */
  endDate?: string;
  /** IANA timezone used for date filtering and calendar buckets. */
  timezone: string;
}

export type SessionHistoryAvailability = 'available' | 'unavailable';
export type SessionHistoryUnavailableReason =
  | 'unsupported'
  | 'recorder-disabled'
  | 'no-recorded-samples'
  | 'database-unavailable'
  | 'invalid-request'
  | 'unknown';

export type SessionBoundaryReason =
  | 'state-change'
  | 'ac-change'
  | 'battery-removed'
  | 'rebooted'
  | 'suspended'
  | 'sampling-gap'
  | 'end-of-data'
  | 'unknown';
export type SessionCompleteness = 'complete' | 'incomplete' | 'unknown';
export type CalendarSummaryPeriod = 'daily' | 'weekly' | 'monthly';

/** Exact camelCase session payload returned by `get_battery_session_history`. */
export interface BatterySessionResponseDto {
  id: string;
  batteryId: string | null;
  state: BatteryState;
  startedAt: string | null;
  endedAt: string | null;
  durationSeconds: number | null;
  startPercentage: number | null;
  endPercentage: number | null;
  startEnergyWh: number | null;
  endEnergyWh: number | null;
  transferredEnergyWh: number | null;
  averagePowerWatts: number | null;
  peakPowerWatts: number | null;
  completeness: SessionCompleteness;
  boundaryReason: SessionBoundaryReason;
}

/** One timezone-aware calendar bucket; missing observations stay null. */
export interface CalendarSummaryResponseDto {
  period: CalendarSummaryPeriod;
  /** Local bucket identifier: date, ISO week, or month for the given timezone. */
  bucket: string;
  timezone: string;
  batteryId: string | null;
  observedEnergyUsedWh: number | null;
  observedEnergyChargedWh: number | null;
  minimumPercentage: number | null;
  maximumPercentage: number | null;
  representativeFullEnergyWh: number | null;
  coverageSeconds: number | null;
  coverageRatio: number | null;
  observedSamples: number;
}

export interface BatterySessionHistoryResponseDto {
  schemaVersion: 1;
  availability: SessionHistoryAvailability;
  unavailableReason: SessionHistoryUnavailableReason | null;
  generatedAt: string | null;
  timezone: string;
  sessions: readonly BatterySessionResponseDto[];
  daily: readonly CalendarSummaryResponseDto[];
  weekly: readonly CalendarSummaryResponseDto[];
  monthly: readonly CalendarSummaryResponseDto[];
}

export interface BatterySessionHistoryData {
  availability: SessionHistoryAvailability;
  unavailableReason: SessionHistoryUnavailableReason | null;
  generatedAt: string | null;
  timezone: string;
  sessions: readonly BatterySessionResponseDto[];
  daily: readonly CalendarSummaryResponseDto[];
  weekly: readonly CalendarSummaryResponseDto[];
  monthly: readonly CalendarSummaryResponseDto[];
}

/** The rebuild is safe to repeat and never mutates immutable raw samples. */
export interface SessionHistoryRebuildResponseDto {
  schemaVersion: 1;
  availability: SessionHistoryAvailability;
  unavailableReason: SessionHistoryUnavailableReason | null;
  rebuiltAt: string | null;
  sessionsRebuilt: number | null;
}

export interface SessionHistoryRebuildResult {
  availability: SessionHistoryAvailability;
  unavailableReason: SessionHistoryUnavailableReason | null;
  rebuiltAt: string | null;
  sessionsRebuilt: number | null;
}

export interface SessionHistoryClient {
  getHistory(request: SessionHistoryRequest): Promise<BatterySessionHistoryData>;
  rebuild(): Promise<SessionHistoryRebuildResult>;
}

export interface SessionHistoryCommandInvoker {
  <Response>(
    command: 'get_battery_session_history',
    arguments_: {
      batteryId?: string;
      states?: readonly BatteryState[];
      startDate?: string;
      endDate?: string;
      timezone: string;
    },
  ): Promise<Response>;
  <Response>(
    command: 'rebuild_battery_session_history',
    arguments_: Record<string, never>,
  ): Promise<Response>;
}

export function createSessionHistoryClient(
  invoke: SessionHistoryCommandInvoker,
): SessionHistoryClient {
  return {
    async getHistory(request) {
      return normalizeSessionHistoryResponse(
        await invoke<BatterySessionHistoryResponseDto>(
          'get_battery_session_history',
          toHistoryCommandArguments(request),
        ),
      );
    },
    async rebuild() {
      return normalizeSessionHistoryRebuildResponse(
        await invoke<SessionHistoryRebuildResponseDto>(
          'rebuild_battery_session_history',
          {},
        ),
      );
    },
  };
}

/** Browser previews must not manufacture session or calendar history. */
export function createDesktopSessionHistoryClient(): SessionHistoryClient {
  if (!isTauriRuntime()) return createUnsupportedSessionHistoryClient();

  return {
    async getHistory(request) {
      const { invoke } = await import('@tauri-apps/api/core');
      return normalizeSessionHistoryResponse(
        await invoke<BatterySessionHistoryResponseDto>(
          'get_battery_session_history',
          toHistoryCommandArguments(request),
        ),
      );
    },
    async rebuild() {
      const { invoke } = await import('@tauri-apps/api/core');
      return normalizeSessionHistoryRebuildResponse(
        await invoke<SessionHistoryRebuildResponseDto>(
          'rebuild_battery_session_history',
          {},
        ),
      );
    },
  };
}

export function createUnsupportedSessionHistoryClient(): SessionHistoryClient {
  return {
    async getHistory(request) {
      return unsupportedHistory(request.timezone);
    },
    async rebuild() {
      return unsupportedRebuild();
    },
  };
}

export function normalizeSessionHistoryResponse(
  response: BatterySessionHistoryResponseDto,
): BatterySessionHistoryData {
  return {
    availability: response.availability,
    unavailableReason: response.unavailableReason,
    generatedAt: response.generatedAt,
    timezone: response.timezone,
    sessions: response.sessions.map((session) => ({ ...session })),
    daily: response.daily.map((summary) => ({ ...summary })),
    weekly: response.weekly.map((summary) => ({ ...summary })),
    monthly: response.monthly.map((summary) => ({ ...summary })),
  };
}

export function normalizeSessionHistoryRebuildResponse(
  response: SessionHistoryRebuildResponseDto,
): SessionHistoryRebuildResult {
  return {
    availability: response.availability,
    unavailableReason: response.unavailableReason,
    rebuiltAt: response.rebuiltAt,
    sessionsRebuilt: response.sessionsRebuilt,
  };
}

function toHistoryCommandArguments(request: SessionHistoryRequest) {
  return {
    ...(request.batteryId === undefined ? {} : { batteryId: request.batteryId }),
    ...(request.states === undefined ? {} : { states: request.states }),
    ...(request.startDate === undefined ? {} : { startDate: request.startDate }),
    ...(request.endDate === undefined ? {} : { endDate: request.endDate }),
    timezone: request.timezone,
  };
}

function unsupportedHistory(timezone: string): BatterySessionHistoryData {
  return {
    availability: 'unavailable',
    unavailableReason: 'unsupported',
    generatedAt: null,
    timezone,
    sessions: [],
    daily: [],
    weekly: [],
    monthly: [],
  };
}

function unsupportedRebuild(): SessionHistoryRebuildResult {
  return {
    availability: 'unavailable',
    unavailableReason: 'unsupported',
    rebuiltAt: null,
    sessionsRebuilt: null,
  };
}

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
