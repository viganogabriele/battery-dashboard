/**
 * Contract for the opt-in local recorder managed by the future Tauri commands.
 *
 * `lastRecordedAt` is deliberately informational: its absence does not imply a
 * database failure or that historical data was deleted. The recorder owns that
 * distinction and reports it through `health`/`error` instead.
 */
export interface RecorderStatusResponseDto {
  schemaVersion: 1;
  supported: boolean;
  enabled: boolean;
  transition: 'idle' | 'enabling' | 'disabling';
  health: 'healthy' | 'unknown' | 'error';
  lastRecordedAt: string | null;
  error: string | null;
}

export type RecorderState =
  'unsupported' | 'disabled' | 'enabling' | 'disabling' | 'enabled' | 'error';

export interface RecorderStatus {
  state: RecorderState;
  lastRecordedAt: string | null;
  error: string | null;
}

/** A small command boundary that keeps Svelte independent from Tauri. */
export interface RecorderClient {
  getStatus(): Promise<RecorderStatus>;
  setEnabled(enabled: boolean): Promise<RecorderStatus>;
}

export interface RecorderCommandInvoker {
  <Response>(command: 'get_recorder_status'): Promise<Response>;
  <Response>(
    command: 'set_recorder_enabled',
    arguments_: { enabled: boolean },
  ): Promise<Response>;
}

export function createRecorderClient(invoke: RecorderCommandInvoker): RecorderClient {
  return {
    async getStatus() {
      return normalizeRecorderStatus(
        await invoke<RecorderStatusResponseDto>('get_recorder_status'),
      );
    },
    async setEnabled(enabled) {
      return normalizeRecorderStatus(
        await invoke<RecorderStatusResponseDto>('set_recorder_enabled', {
          enabled,
        }),
      );
    },
  };
}

/**
 * A browser-safe adapter for the dashboard preview. It never attempts to call
 * a desktop command outside Tauri and therefore does not turn preview mode
 * into a misleading recorder error.
 */
export function createDesktopRecorderClient(): RecorderClient {
  if (!isTauriRuntime()) return createUnsupportedRecorderClient();

  return {
    async getStatus() {
      const { invoke } = await import('@tauri-apps/api/core');
      return normalizeRecorderStatus(
        await invoke<RecorderStatusResponseDto>('get_recorder_status'),
      );
    },
    async setEnabled(enabled) {
      const { invoke } = await import('@tauri-apps/api/core');
      return normalizeRecorderStatus(
        await invoke<RecorderStatusResponseDto>('set_recorder_enabled', { enabled }),
      );
    },
  };
}

export function createUnsupportedRecorderClient(): RecorderClient {
  const status: RecorderStatus = {
    state: 'unsupported',
    lastRecordedAt: null,
    error: null,
  };

  return {
    async getStatus() {
      return status;
    },
    async setEnabled() {
      return status;
    },
  };
}

export function normalizeRecorderStatus(
  response: RecorderStatusResponseDto,
): RecorderStatus {
  if (!response.supported) {
    return { state: 'unsupported', lastRecordedAt: null, error: null };
  }

  if (response.transition === 'enabling') {
    return { state: 'enabling', lastRecordedAt: response.lastRecordedAt, error: null };
  }

  if (response.transition === 'disabling') {
    return { state: 'disabling', lastRecordedAt: response.lastRecordedAt, error: null };
  }

  if (response.health === 'error' || response.error !== null) {
    return {
      state: 'error',
      lastRecordedAt: response.lastRecordedAt,
      error: response.error,
    };
  }

  return {
    state: response.enabled ? 'enabled' : 'disabled',
    lastRecordedAt: response.lastRecordedAt,
    error: null,
  };
}

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}
