import { describe, expect, it, vi } from 'vitest';

import {
  createDesktopRuntimeForecastClient,
  createRuntimeForecastClient,
  createUnsupportedRuntimeForecastClient,
  type RuntimeForecastResponseDto,
} from './runtime-forecast-client';

const generatedAt = '2026-08-24T09:00:00.000Z';

function response(
  overrides: Partial<RuntimeForecastResponseDto> = {},
): RuntimeForecastResponseDto {
  return {
    schemaVersion: 1,
    availability: 'available',
    unavailableReason: null,
    generatedAt,
    batteryId: 'BAT0',
    state: 'discharging',
    bandStartPercent: 40,
    bandEndPercent: 60,
    evidence: 'sufficient',
    confidence: 'moderate',
    sessionCount: 4,
    historicalRatePercentPerHour: 12,
    liveRatePercentPerHour: 14,
    liveRateWindowMinutes: 20,
    blendedRatePercentPerHour: 12.8,
    estimatedMinutesRemaining: 234,
    estimatedAt: '2026-08-24T13:00:00.000Z',
    ...overrides,
  };
}

describe('runtime-forecast-client', () => {
  it('passes an explicit battery id, state, and current percentage', async () => {
    const invoke = vi.fn().mockResolvedValue(response());
    const client = createRuntimeForecastClient(invoke);

    await client.getRuntimeForecast({
      batteryId: 'BAT1',
      state: 'discharging',
      currentPercentage: 55,
    });
    await client.getRuntimeForecast({ state: 'charging', currentPercentage: 30 });

    expect(invoke).toHaveBeenNthCalledWith(1, 'get_runtime_forecast', {
      batteryId: 'BAT1',
      state: 'discharging',
      currentPercentage: 55,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, 'get_runtime_forecast', {
      state: 'charging',
      currentPercentage: 30,
    });
  });

  it('returns the forecast unchanged, preserving evidence and the distinct live/historical rates', async () => {
    const invoke = vi.fn().mockResolvedValue(response());
    const client = createRuntimeForecastClient(invoke);

    const forecast = await client.getRuntimeForecast({
      state: 'discharging',
      currentPercentage: 55,
    });

    expect(forecast.evidence).toBe('sufficient');
    expect(forecast.historicalRatePercentPerHour).toBe(12);
    expect(forecast.liveRatePercentPerHour).toBe(14);
    expect(forecast.blendedRatePercentPerHour).toBe(12.8);
    expect(forecast.estimatedAt).toBe('2026-08-24T13:00:00.000Z');
  });

  it('returns an unsupported, explicitly insufficient forecast outside Tauri', async () => {
    const client = createDesktopRuntimeForecastClient();

    const forecast = await client.getRuntimeForecast({
      state: 'discharging',
      currentPercentage: 55,
    });

    expect(forecast.availability).toBe('unavailable');
    expect(forecast.unavailableReason).toBe('unsupported');
    expect(forecast.evidence).toBe('insufficient');
    expect(forecast.estimatedMinutesRemaining).toBeNull();
  });

  it('the plain unsupported client never fabricates a number either', async () => {
    const client = createUnsupportedRuntimeForecastClient();

    const forecast = await client.getRuntimeForecast({
      state: 'charging',
      currentPercentage: 10,
    });

    expect(forecast.availability).toBe('unavailable');
    expect(forecast.state).toBe('charging');
    expect(forecast.estimatedAt).toBeNull();
  });
});
