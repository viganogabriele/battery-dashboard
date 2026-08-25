import { describe, expect, it, vi } from 'vitest';

import {
  createDayUsageClient,
  createDesktopDayUsageClient,
  normalizeDayUsageResponse,
  type DayUsageResponseDto,
  type TodayVsYesterdayResponseDto,
} from './day-usage-client';

const generatedAt = '2026-08-24T09:00:00.000Z';

function sufficientDay(
  overrides: Partial<DayUsageResponseDto> = {},
): DayUsageResponseDto {
  return {
    available: true,
    date: '2026-08-23',
    dayStart: '2026-08-22T22:00:00.000Z',
    dayEnd: '2026-08-23T22:00:00.000Z',
    evidence: 'sufficient',
    insufficientReason: null,
    sampleCount: 640,
    elapsedSeconds: 86400,
    observedDurationSeconds: 82000,
    coverageRatio: 0.949,
    startPercentage: 92,
    endPercentage: 41,
    percentageChange: -51,
    energyChangeWh: -22.4,
    averageDischargePowerWatts: 6.8,
    averageChargePowerWatts: null,
    contributingBatteries: null,
    ...overrides,
  };
}

function insufficientDay(): DayUsageResponseDto {
  return {
    available: true,
    date: '2026-08-24',
    dayStart: '2026-08-23T22:00:00.000Z',
    dayEnd: '2026-08-24T09:00:00.000Z',
    evidence: 'insufficient',
    insufficientReason: 'no-recording',
    sampleCount: 0,
    elapsedSeconds: 39600,
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

function response(
  overrides: Partial<TodayVsYesterdayResponseDto> = {},
): TodayVsYesterdayResponseDto {
  return {
    schemaVersion: 1,
    availability: 'available',
    unavailableReason: null,
    generatedAt,
    timezone: 'Europe/Rome',
    batteryId: 'BAT0',
    today: insufficientDay(),
    yesterday: sufficientDay(),
    ...overrides,
  };
}

describe('day-usage-client', () => {
  it('preserves per-day evidence, insufficiency reasons, and null derived fields', () => {
    const comparison = normalizeDayUsageResponse(response());

    expect(comparison.yesterday).toMatchObject({
      evidence: 'sufficient',
      percentageChange: -51,
      energyChangeWh: -22.4,
    });
    expect(comparison.today).toMatchObject({
      evidence: 'insufficient',
      insufficientReason: 'no-recording',
      sampleCount: 0,
    });
    expect(comparison.today.percentageChange).toBeNull();
    expect(comparison.today.percentageChange).not.toBe(0);
  });

  it('copies each day instead of exposing the mutable response DTO', () => {
    const source = response();
    const comparison = normalizeDayUsageResponse(source);

    expect(comparison.today).not.toBe(source.today);
    expect(comparison.yesterday).not.toBe(source.yesterday);
  });

  it('passes an explicit battery id, omits it for the aggregate view, and forwards the timezone', async () => {
    const invoke = vi.fn().mockResolvedValue(response());
    const client = createDayUsageClient(invoke);

    await client.getTodayVsYesterday({
      batteryId: 'BAT1',
      timezone: 'America/New_York',
    });
    await client.getTodayVsYesterday({ timezone: 'Europe/Rome' });

    expect(invoke).toHaveBeenNthCalledWith(1, 'get_today_vs_yesterday_usage', {
      batteryId: 'BAT1',
      timezone: 'America/New_York',
    });
    expect(invoke).toHaveBeenNthCalledWith(2, 'get_today_vs_yesterday_usage', {
      timezone: 'Europe/Rome',
    });
  });

  it('returns an unavailable, explicitly insufficient comparison outside Tauri', async () => {
    const client = createDesktopDayUsageClient();

    const comparison = await client.getTodayVsYesterday({ timezone: 'UTC' });

    expect(comparison.availability).toBe('unavailable');
    expect(comparison.unavailableReason).toBe('unsupported');
    expect(comparison.today.evidence).toBe('insufficient');
    expect(comparison.yesterday.evidence).toBe('insufficient');
    expect(comparison.today.percentageChange).toBeNull();
  });
});
