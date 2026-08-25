import { describe, expect, it, vi } from 'vitest';

import {
  createDesktopRecentBatteryHistoryClient,
  createRecentBatteryHistoryClient,
  normalizeRecentBatteryHistoryResponse,
  type HistoryMetricResponseDto,
  type RecentBatteryHistoryResponseDto,
} from './recent-history-client';

const timestamp = '2026-08-23T12:00:00.000Z';

function available(
  value: number,
  source: HistoryMetricResponseDto['source'] = 'upower',
): HistoryMetricResponseDto {
  return { value, source, availability: 'available', observedAt: timestamp };
}

function unavailable(): HistoryMetricResponseDto {
  return {
    value: null,
    source: 'unavailable',
    availability: 'unavailable',
    observedAt: null,
  };
}

function numericSummary(value: number) {
  return {
    minimum: value - 1,
    maximum: value + 1,
    average: value,
    minimumAt: '2026-08-23T11:58:00.000Z',
    maximumAt: '2026-08-23T11:59:00.000Z',
    observedSamples: 3,
    source: 'derived' as const,
    availability: 'available' as const,
    observedAt: timestamp,
  };
}

function response(
  overrides: Partial<RecentBatteryHistoryResponseDto> = {},
): RecentBatteryHistoryResponseDto {
  return {
    schemaVersion: 1,
    availability: 'available',
    unavailableReason: null,
    source: 'sqlite',
    freshness: 'fresh',
    batteryId: 'BAT0',
    rangeHours: 6,
    collectedAt: timestamp,
    points: [
      {
        batteryId: 'BAT0',
        recordedAt: timestamp,
        kind: 'persisted',
        state: 'discharging',
        freshness: 'fresh',
        metrics: {
          percentage: available(63),
          energyNowWh: available(35.9, 'sysfs'),
          powerWatts: available(-8.4),
        },
      },
      {
        batteryId: 'BAT0',
        recordedAt: '2026-08-23T12:01:00.000Z',
        kind: 'transient',
        state: 'discharging',
        freshness: 'fresh',
        metrics: {
          percentage: available(62),
          energyNowWh: available(35.8, 'upower'),
          powerWatts: available(-8.7),
        },
      },
    ],
    gaps: [],
    summary: {
      percentage: numericSummary(62.5),
      powerWatts: numericSummary(-8.55),
      energyNowWh: numericSummary(35.85),
      observedEnergyWh: {
        first: 35.9,
        last: 35.8,
        change: -0.1,
        observedSamples: 2,
        source: 'derived',
        availability: 'available',
        observedAt: '2026-08-23T12:01:00.000Z',
      },
    },
    ...overrides,
  };
}

describe('normalizeRecentBatteryHistoryResponse', () => {
  it('preserves persisted and explicitly transient points with their providers', () => {
    const history = normalizeRecentBatteryHistoryResponse(response());

    expect(history.source).toBe('sqlite');
    expect(history.points.map((point) => point.kind)).toEqual([
      'persisted',
      'transient',
    ]);
    expect(history.points[0]?.energyNowWh.source).toBe('sysfs');
    expect(history.points[1]?.powerWatts.value).toBe(-8.7);
    expect(history.summary.observedEnergyWh.change).toBe(-0.1);
  });

  it('preserves when the recorded minimum and maximum were observed', () => {
    const history = normalizeRecentBatteryHistoryResponse(response());

    expect(history.summary.percentage.minimumAt).toBe('2026-08-23T11:58:00.000Z');
    expect(history.summary.percentage.maximumAt).toBe('2026-08-23T11:59:00.000Z');
  });

  it('preserves data gaps and unavailable readings without manufacturing zeroes', () => {
    const source = response();
    const firstPoint = source.points[0];
    if (!firstPoint) throw new Error('test fixture is missing a point');
    const history = normalizeRecentBatteryHistoryResponse({
      ...source,
      freshness: 'stale',
      points: [
        {
          ...firstPoint,
          freshness: 'stale',
          metrics: { ...firstPoint.metrics, powerWatts: unavailable() },
        },
      ],
      gaps: [
        {
          startsAt: '2026-08-23T10:00:00.000Z',
          endsAt: '2026-08-23T11:00:00.000Z',
          reason: 'suspended',
          detail: null,
        },
      ],
    });

    expect(history.freshness).toBe('stale');
    expect(history.points[0]?.powerWatts).toEqual({
      value: null,
      source: 'unavailable',
      availability: 'unavailable',
      updatedAt: null,
    });
    expect(history.points[0]?.powerWatts.value).not.toBe(0);
    expect(history.gaps).toEqual([
      {
        startsAt: '2026-08-23T10:00:00.000Z',
        endsAt: '2026-08-23T11:00:00.000Z',
        reason: 'suspended',
        detail: null,
      },
    ]);
  });

  it.each(['charging', 'discharging', 'full', 'idle', 'unknown'] as const)(
    'preserves the recorded %s state',
    (state) => {
      const source = response();
      const firstPoint = source.points[0];
      if (!firstPoint) throw new Error('test fixture is missing a point');
      const history = normalizeRecentBatteryHistoryResponse({
        ...source,
        points: [{ ...firstPoint, state }],
      });

      expect(history.points[0]?.state).toBe(state);
    },
  );

  it('marks partial summaries unavailable instead of inventing min/max/average', () => {
    const source = response();
    const history = normalizeRecentBatteryHistoryResponse({
      ...source,
      summary: {
        ...source.summary,
        powerWatts: {
          ...source.summary.powerWatts,
          average: null,
          observedSamples: 1,
        },
        observedEnergyWh: {
          ...source.summary.observedEnergyWh,
          change: null,
          observedSamples: 1,
        },
      },
    });

    expect(history.summary.powerWatts).toMatchObject({
      minimum: null,
      maximum: null,
      average: null,
      observedSamples: 1,
      availability: 'unavailable',
    });
    expect(history.summary.observedEnergyWh).toMatchObject({
      first: null,
      last: null,
      change: null,
      observedSamples: 1,
      availability: 'unavailable',
    });
  });

  it('passes the exact Tauri command and omits an unspecified battery id', async () => {
    const invoke = vi.fn().mockResolvedValue(response());
    const client = createRecentBatteryHistoryClient(invoke);

    await client.getRecentHistory({ rangeHours: 6, maxPoints: 120 });
    await client.getRecentHistory({
      batteryId: 'BAT1',
      rangeHours: 24,
      maxPoints: 480,
    });

    expect(invoke).toHaveBeenNthCalledWith(1, 'get_recent_battery_history', {
      rangeHours: 6,
      maxPoints: 120,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, 'get_recent_battery_history', {
      batteryId: 'BAT1',
      rangeHours: 24,
      maxPoints: 480,
    });
  });

  it('passes through the multi-day range windows unchanged', async () => {
    const invoke = vi.fn().mockResolvedValue(response());
    const client = createRecentBatteryHistoryClient(invoke);

    await client.getRecentHistory({ rangeHours: 168, maxPoints: 720 });

    expect(invoke).toHaveBeenCalledWith('get_recent_battery_history', {
      rangeHours: 168,
      maxPoints: 720,
    });
  });

  it('returns unsupported and empty browser history instead of fixture data', async () => {
    const client = createDesktopRecentBatteryHistoryClient();
    const history = await client.getRecentHistory({ rangeHours: 2, maxPoints: 60 });

    expect(history).toMatchObject({
      availability: 'unavailable',
      unavailableReason: 'unsupported',
      source: 'unavailable',
      points: [],
    });
    expect(history.summary.percentage.average).toBeNull();
  });
});
