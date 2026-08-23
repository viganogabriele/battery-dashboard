import { describe, expect, it, vi } from 'vitest';

import {
  createDesktopSessionHistoryClient,
  createSessionHistoryClient,
  normalizeSessionHistoryResponse,
  type BatterySessionHistoryResponseDto,
} from './session-history-client';

const timestamp = '2026-08-23T12:00:00.000Z';

function response(
  overrides: Partial<BatterySessionHistoryResponseDto> = {},
): BatterySessionHistoryResponseDto {
  return {
    schemaVersion: 1,
    availability: 'available',
    unavailableReason: null,
    generatedAt: timestamp,
    timezone: 'Europe/Rome',
    sessions: [
      {
        id: 'BAT0:1',
        batteryId: 'BAT0',
        state: 'discharging',
        startedAt: '2026-08-23T10:00:00.000Z',
        endedAt: timestamp,
        durationSeconds: 7200,
        startPercentage: 78,
        endPercentage: 63,
        startEnergyWh: 44.4,
        endEnergyWh: 35.9,
        transferredEnergyWh: -8.5,
        averagePowerWatts: -4.25,
        peakPowerWatts: -8.7,
        completeness: 'incomplete',
        boundaryReason: 'end-of-data',
      },
    ],
    daily: [
      {
        period: 'daily',
        bucket: '2026-08-23',
        timezone: 'Europe/Rome',
        batteryId: 'BAT0',
        observedEnergyUsedWh: 8.5,
        observedEnergyChargedWh: null,
        minimumPercentage: 63,
        maximumPercentage: 78,
        representativeFullEnergyWh: null,
        coverageSeconds: 7200,
        coverageRatio: 0.0833,
        observedSamples: 121,
      },
    ],
    weekly: [],
    monthly: [],
    ...overrides,
  };
}

describe('session-history-client', () => {
  it('preserves incomplete sessions and null calendar observations', () => {
    const history = normalizeSessionHistoryResponse(response());

    expect(history.sessions[0]).toMatchObject({
      state: 'discharging',
      completeness: 'incomplete',
      boundaryReason: 'end-of-data',
      transferredEnergyWh: -8.5,
    });
    expect(history.daily[0]).toMatchObject({
      bucket: '2026-08-23',
      observedEnergyChargedWh: null,
      representativeFullEnergyWh: null,
      coverageRatio: 0.0833,
    });
    expect(history.daily[0]?.observedEnergyChargedWh).not.toBe(0);
  });

  it('copies response collections instead of exposing mutable DTO arrays', () => {
    const source = response();
    const history = normalizeSessionHistoryResponse(source);
    const sourceSession = source.sessions[0];
    const normalizedSession = history.sessions[0];
    const sourceDaily = source.daily[0];
    const normalizedDaily = history.daily[0];

    expect(history.sessions).not.toBe(source.sessions);
    expect(normalizedSession).not.toBe(sourceSession);
    expect(normalizedDaily).not.toBe(sourceDaily);
  });

  it('passes filters exactly, omits unspecified filters, and invokes the idempotent rebuild command', async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(response())
      .mockResolvedValueOnce(response())
      .mockResolvedValueOnce({
        schemaVersion: 1,
        availability: 'available',
        unavailableReason: null,
        rebuiltAt: timestamp,
        sessionsRebuilt: 4,
      });
    const client = createSessionHistoryClient(invoke);

    await client.getHistory({ timezone: 'Europe/Rome' });
    await client.getHistory({
      batteryId: 'BAT1',
      states: ['charging', 'full'],
      startDate: '2026-08-01',
      endDate: '2026-08-23',
      timezone: 'America/New_York',
    });
    const rebuilt = await client.rebuild();

    expect(invoke).toHaveBeenNthCalledWith(1, 'get_battery_session_history', {
      timezone: 'Europe/Rome',
    });
    expect(invoke).toHaveBeenNthCalledWith(2, 'get_battery_session_history', {
      batteryId: 'BAT1',
      states: ['charging', 'full'],
      startDate: '2026-08-01',
      endDate: '2026-08-23',
      timezone: 'America/New_York',
    });
    expect(invoke).toHaveBeenNthCalledWith(3, 'rebuild_battery_session_history', {});
    expect(rebuilt).toEqual({
      availability: 'available',
      unavailableReason: null,
      rebuiltAt: timestamp,
      sessionsRebuilt: 4,
    });
  });

  it('returns unavailable empty browser data and never pretends to rebuild', async () => {
    const client = createDesktopSessionHistoryClient();
    const history = await client.getHistory({ timezone: 'UTC' });
    const rebuilt = await client.rebuild();

    expect(history).toEqual({
      availability: 'unavailable',
      unavailableReason: 'unsupported',
      generatedAt: null,
      timezone: 'UTC',
      sessions: [],
      daily: [],
      weekly: [],
      monthly: [],
    });
    expect(rebuilt).toEqual({
      availability: 'unavailable',
      unavailableReason: 'unsupported',
      rebuiltAt: null,
      sessionsRebuilt: null,
    });
  });
});
