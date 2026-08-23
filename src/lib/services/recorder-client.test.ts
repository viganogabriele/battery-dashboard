import { describe, expect, it, vi } from 'vitest';

import {
  createDesktopRecorderClient,
  createRecorderClient,
  normalizeRecorderStatus,
  type RecorderStatusResponseDto,
} from './recorder-client';

const healthy = (
  overrides: Partial<RecorderStatusResponseDto> = {},
): RecorderStatusResponseDto => ({
  schemaVersion: 1,
  supported: true,
  enabled: false,
  transition: 'idle',
  health: 'healthy',
  lastRecordedAt: null,
  error: null,
  ...overrides,
});

describe('normalizeRecorderStatus', () => {
  it.each([
    [healthy({ supported: false }), 'unsupported'],
    [healthy(), 'disabled'],
    [healthy({ enabled: true }), 'enabled'],
    [healthy({ transition: 'enabling' }), 'enabling'],
    [healthy({ transition: 'disabling', enabled: true }), 'disabling'],
    [healthy({ health: 'error', error: 'Timer unit could not start' }), 'error'],
  ] as const)('maps recorder state %s safely', (response, state) => {
    expect(normalizeRecorderStatus(response).state).toBe(state);
  });

  it('passes both command names and the requested enabled value through', async () => {
    const invoke = vi.fn().mockResolvedValue(healthy({ enabled: true }));
    const client = createRecorderClient(invoke);

    await client.getStatus();
    await client.setEnabled(true);

    expect(invoke).toHaveBeenNthCalledWith(1, 'get_recorder_status');
    expect(invoke).toHaveBeenNthCalledWith(2, 'set_recorder_enabled', {
      enabled: true,
    });
  });

  it('uses an unsupported status in a browser preview without invoking Tauri', async () => {
    const client = createDesktopRecorderClient();

    await expect(client.getStatus()).resolves.toMatchObject({
      state: 'unsupported',
    });
  });
});
