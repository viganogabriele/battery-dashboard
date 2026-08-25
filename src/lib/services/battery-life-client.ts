/** "How long does my battery last on a full charge" — built exclusively from
 * completed discharge sessions already recorded in local SQLite history.
 * Nothing here is interpolated or extrapolated beyond a session's own
 * directly recorded start percentage, end percentage, and duration. */

export type BatteryLifeAvailability = 'available' | 'unavailable';
export type BatteryLifeUnavailableReason =
  | 'unsupported'
  | 'recorder-disabled'
  | 'no-recorded-samples'
  | 'database-unavailable'
  | 'invalid-request'
  | 'unknown';

export type BatteryLifeEvidence = 'sufficient' | 'insufficient';
export type BatteryLifeConfidence = 'none' | 'low' | 'moderate' | 'high';

/** Exact camelCase payload from `get_battery_life_estimate`. */
export interface BatteryLifeHeadlineDto {
  evidence: BatteryLifeEvidence;
  confidence: BatteryLifeConfidence;
  sessionCount: number;
  averageMinutes: number | null;
  medianMinutes: number | null;
  minMinutes: number | null;
  maxMinutes: number | null;
}

export interface DurationStatsDto {
  count: number;
  averageMinutes: number;
  medianMinutes: number;
  minMinutes: number;
  maxMinutes: number;
}

export interface StartingChargeBandDto {
  bandStartPercent: number;
  bandEndPercent: number;
  isFullChargeBand: boolean;
  allSessions: DurationStatsDto | null;
  fullyDrained: DurationStatsDto | null;
}

export interface BatteryLifeResponseDto {
  schemaVersion: 1;
  availability: BatteryLifeAvailability;
  unavailableReason: BatteryLifeUnavailableReason | null;
  generatedAt: string | null;
  batteryId: string | null;
  fullChargeMinPercent: number;
  fullyDrainedMaxPercent: number;
  headline: BatteryLifeHeadlineDto;
  bands: StartingChargeBandDto[];
  totalSessionCount: number;
  earliestSessionStartedAt: string | null;
  latestSessionEndedAt: string | null;
}

export interface BatteryLifeEstimate {
  availability: BatteryLifeAvailability;
  unavailableReason: BatteryLifeUnavailableReason | null;
  generatedAt: string | null;
  batteryId: string | null;
  fullChargeMinPercent: number;
  fullyDrainedMaxPercent: number;
  headline: BatteryLifeHeadlineDto;
  bands: StartingChargeBandDto[];
  totalSessionCount: number;
  earliestSessionStartedAt: string | null;
  latestSessionEndedAt: string | null;
}

export interface BatteryLifeRequest {
  /** Omit for the aggregate "all batteries" view. */
  batteryId?: string;
}

export interface BatteryLifeClient {
  getBatteryLifeEstimate(request: BatteryLifeRequest): Promise<BatteryLifeEstimate>;
}

export interface BatteryLifeCommandInvoker {
  <Response>(
    command: 'get_battery_life_estimate',
    arguments_: { batteryId?: string },
  ): Promise<Response>;
}

export function createBatteryLifeClient(
  invoke: BatteryLifeCommandInvoker,
): BatteryLifeClient {
  return {
    async getBatteryLifeEstimate(request) {
      return normalizeBatteryLifeResponse(
        await invoke<BatteryLifeResponseDto>(
          'get_battery_life_estimate',
          toCommandArguments(request),
        ),
      );
    },
  };
}

/** Browser previews must not manufacture a battery-life estimate. */
export function createDesktopBatteryLifeClient(): BatteryLifeClient {
  if (!isTauriRuntime()) return createUnsupportedBatteryLifeClient();

  return {
    async getBatteryLifeEstimate(request) {
      const { invoke } = await import('@tauri-apps/api/core');
      return normalizeBatteryLifeResponse(
        await invoke<BatteryLifeResponseDto>(
          'get_battery_life_estimate',
          toCommandArguments(request),
        ),
      );
    },
  };
}

export function createUnsupportedBatteryLifeClient(): BatteryLifeClient {
  return {
    async getBatteryLifeEstimate() {
      return unsupportedEstimate();
    },
  };
}

export function normalizeBatteryLifeResponse(
  response: BatteryLifeResponseDto,
): BatteryLifeEstimate {
  return {
    availability: response.availability,
    unavailableReason: response.unavailableReason,
    generatedAt: response.generatedAt,
    batteryId: response.batteryId,
    fullChargeMinPercent: response.fullChargeMinPercent,
    fullyDrainedMaxPercent: response.fullyDrainedMaxPercent,
    headline: { ...response.headline },
    bands: response.bands.map((band) => ({ ...band })),
    totalSessionCount: response.totalSessionCount,
    earliestSessionStartedAt: response.earliestSessionStartedAt,
    latestSessionEndedAt: response.latestSessionEndedAt,
  };
}

function toCommandArguments(request: BatteryLifeRequest) {
  return {
    ...(request.batteryId === undefined ? {} : { batteryId: request.batteryId }),
  };
}

function unavailableHeadline(): BatteryLifeHeadlineDto {
  return {
    evidence: 'insufficient',
    confidence: 'none',
    sessionCount: 0,
    averageMinutes: null,
    medianMinutes: null,
    minMinutes: null,
    maxMinutes: null,
  };
}

function unsupportedEstimate(): BatteryLifeEstimate {
  return {
    availability: 'unavailable',
    unavailableReason: 'unsupported',
    generatedAt: null,
    batteryId: null,
    fullChargeMinPercent: 95,
    fullyDrainedMaxPercent: 20,
    headline: unavailableHeadline(),
    bands: [],
    totalSessionCount: 0,
    earliestSessionStartedAt: null,
    latestSessionEndedAt: null,
  };
}

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
