import { describe, expect, it, vi } from 'vitest';

import {
  createBatteryLifeClient,
  createDesktopBatteryLifeClient,
  normalizeBatteryLifeResponse,
  type BatteryLifeResponseDto,
  type StartingChargeBandDto,
} from './battery-life-client';

const generatedAt = '2026-08-24T09:00:00.000Z';

function band(overrides: Partial<StartingChargeBandDto> = {}): StartingChargeBandDto {
  return {
    bandStartPercent: 95,
    bandEndPercent: 100,
    isFullChargeBand: true,
    allSessions: null,
    fullyDrained: null,
    ...overrides,
  };
}

function response(
  overrides: Partial<BatteryLifeResponseDto> = {},
): BatteryLifeResponseDto {
  return {
    schemaVersion: 1,
    availability: 'available',
    unavailableReason: null,
    generatedAt,
    batteryId: 'BAT0',
    fullChargeMinPercent: 95,
    fullyDrainedMaxPercent: 20,
    headline: {
      evidence: 'sufficient',
      confidence: 'moderate',
      sessionCount: 4,
      averageMinutes: 320,
      medianMinutes: 300,
      minMinutes: 250,
      maxMinutes: 400,
    },
    bands: [
      band({
        allSessions: {
          count: 4,
          averageMinutes: 320,
          medianMinutes: 300,
          minMinutes: 250,
          maxMinutes: 400,
        },
        fullyDrained: {
          count: 4,
          averageMinutes: 320,
          medianMinutes: 300,
          minMinutes: 250,
          maxMinutes: 400,
        },
      }),
      band({ bandStartPercent: 80, bandEndPercent: 95, isFullChargeBand: false }),
    ],
    totalSessionCount: 5,
    earliestSessionStartedAt: '2026-08-01T08:00:00.000Z',
    latestSessionEndedAt: '2026-08-23T12:00:00.000Z',
    ...overrides,
  };
}

describe('battery-life-client', () => {
  it('preserves the headline evidence, confidence, and duration statistics', () => {
    const estimate = normalizeBatteryLifeResponse(response());

    expect(estimate.headline).toMatchObject({
      evidence: 'sufficient',
      confidence: 'moderate',
      sessionCount: 4,
      averageMinutes: 320,
    });
    expect(estimate.bands).toHaveLength(2);
    expect(estimate.bands[0].isFullChargeBand).toBe(true);
    expect(estimate.totalSessionCount).toBe(5);
  });

  it('never fabricates a headline number when evidence is insufficient', () => {
    const estimate = normalizeBatteryLifeResponse(
      response({
        headline: {
          evidence: 'insufficient',
          confidence: 'none',
          sessionCount: 0,
          averageMinutes: null,
          medianMinutes: null,
          minMinutes: null,
          maxMinutes: null,
        },
      }),
    );

    expect(estimate.headline.evidence).toBe('insufficient');
    expect(estimate.headline.averageMinutes).toBeNull();
    expect(estimate.headline.sessionCount).toBe(0);
  });

  it('copies bands instead of exposing the mutable response DTO', () => {
    const source = response();
    const estimate = normalizeBatteryLifeResponse(source);

    expect(estimate.bands).not.toBe(source.bands);
    expect(estimate.headline).not.toBe(source.headline);
  });

  it('passes an explicit battery id and omits it for the aggregate view', async () => {
    const invoke = vi.fn().mockResolvedValue(response());
    const client = createBatteryLifeClient(invoke);

    await client.getBatteryLifeEstimate({ batteryId: 'BAT1' });
    await client.getBatteryLifeEstimate({});

    expect(invoke).toHaveBeenNthCalledWith(1, 'get_battery_life_estimate', {
      batteryId: 'BAT1',
    });
    expect(invoke).toHaveBeenNthCalledWith(2, 'get_battery_life_estimate', {});
  });

  it('returns an unavailable, explicitly insufficient estimate outside Tauri', async () => {
    const client = createDesktopBatteryLifeClient();

    const estimate = await client.getBatteryLifeEstimate({});

    expect(estimate.availability).toBe('unavailable');
    expect(estimate.unavailableReason).toBe('unsupported');
    expect(estimate.headline.evidence).toBe('insufficient');
    expect(estimate.headline.averageMinutes).toBeNull();
    expect(estimate.bands).toEqual([]);
  });
});
