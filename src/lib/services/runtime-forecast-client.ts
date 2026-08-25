/** "Based on how you've actually been using this laptop, and where the
 * battery is right now, about when will it run out (or finish charging)" —
 * a live, current-moment forecast built from this machine's own recorded
 * discharge/charge sessions plus a short recent live trend, entirely
 * distinct from any `UPower`-provided time-to-empty/time-to-full estimate
 * (`DEVELOPMENT_PLAN.md` section 11 keeps those two kinds of estimate
 * explicitly separate). Nothing here is fabricated: insufficient recorded
 * history at the current charge level is reported honestly instead of
 * guessed at. */

export type RuntimeForecastAvailability =
  'available' | 'unavailable' | 'not-applicable';
export type RuntimeForecastUnavailableReason =
  | 'unsupported'
  | 'recorder-disabled'
  | 'no-recorded-samples'
  | 'database-unavailable'
  | 'invalid-request'
  | 'unknown';
export type RuntimeForecastEvidence = 'sufficient' | 'insufficient';
export type RuntimeForecastConfidence = 'none' | 'low' | 'moderate' | 'high';

/** Exact camelCase payload from `get_runtime_forecast`. */
export interface RuntimeForecastResponseDto {
  schemaVersion: 1;
  availability: RuntimeForecastAvailability;
  unavailableReason: RuntimeForecastUnavailableReason | null;
  generatedAt: string | null;
  batteryId: string | null;
  state: string | null;
  bandStartPercent: number | null;
  bandEndPercent: number | null;
  evidence: RuntimeForecastEvidence;
  confidence: RuntimeForecastConfidence;
  sessionCount: number;
  historicalRatePercentPerHour: number | null;
  liveRatePercentPerHour: number | null;
  liveRateWindowMinutes: number | null;
  blendedRatePercentPerHour: number | null;
  estimatedMinutesRemaining: number | null;
  estimatedAt: string | null;
}

export type RuntimeForecast = RuntimeForecastResponseDto;

export interface RuntimeForecastRequest {
  /** Omit for the aggregate "all batteries" view. */
  batteryId?: string;
  /** The selected snapshot's current normalized state. */
  state: string;
  /** The selected snapshot's current charge percentage. */
  currentPercentage: number;
}

export interface RuntimeForecastClient {
  getRuntimeForecast(request: RuntimeForecastRequest): Promise<RuntimeForecast>;
}

export interface RuntimeForecastCommandInvoker {
  <Response>(
    command: 'get_runtime_forecast',
    arguments_: { batteryId?: string; state: string; currentPercentage: number },
  ): Promise<Response>;
}

export function createRuntimeForecastClient(
  invoke: RuntimeForecastCommandInvoker,
): RuntimeForecastClient {
  return {
    async getRuntimeForecast(request) {
      return invoke<RuntimeForecastResponseDto>(
        'get_runtime_forecast',
        toCommandArguments(request),
      );
    },
  };
}

/** Browser previews must not manufacture a runtime forecast. */
export function createDesktopRuntimeForecastClient(): RuntimeForecastClient {
  if (!isTauriRuntime()) return createUnsupportedRuntimeForecastClient();

  return {
    async getRuntimeForecast(request) {
      const { invoke } = await import('@tauri-apps/api/core');
      return invoke<RuntimeForecastResponseDto>(
        'get_runtime_forecast',
        toCommandArguments(request),
      );
    },
  };
}

export function createUnsupportedRuntimeForecastClient(): RuntimeForecastClient {
  return {
    async getRuntimeForecast(request) {
      return unsupportedForecast(request.state);
    },
  };
}

function toCommandArguments(request: RuntimeForecastRequest) {
  return {
    ...(request.batteryId === undefined ? {} : { batteryId: request.batteryId }),
    state: request.state,
    currentPercentage: request.currentPercentage,
  };
}

function unsupportedForecast(state: string): RuntimeForecast {
  return {
    schemaVersion: 1,
    availability: 'unavailable',
    unavailableReason: 'unsupported',
    generatedAt: null,
    batteryId: null,
    state,
    bandStartPercent: null,
    bandEndPercent: null,
    evidence: 'insufficient',
    confidence: 'none',
    sessionCount: 0,
    historicalRatePercentPerHour: null,
    liveRatePercentPerHour: null,
    liveRateWindowMinutes: null,
    blendedRatePercentPerHour: null,
    estimatedMinutesRemaining: null,
    estimatedAt: null,
  };
}

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
