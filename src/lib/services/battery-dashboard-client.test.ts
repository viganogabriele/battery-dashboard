import { describe, expect, it } from 'vitest';

import {
  createFixtureBatteryDashboardClient,
  normalizeBatteryDashboardResponse,
  type BatteryDashboardResponseDto,
  type BatteryMetricResponseDto,
} from './battery-dashboard-client';

const timestamp = '2026-08-23T12:00:00.000Z';

function available(
  value: number,
  source: BatteryMetricResponseDto['source'] = 'upower',
) {
  return { value, source, availability: 'available' as const, updatedAt: timestamp };
}

function unavailable(): BatteryMetricResponseDto {
  return {
    value: null,
    source: 'unavailable',
    availability: 'unavailable',
    updatedAt: null,
  };
}

function response(
  overrides: Partial<BatteryDashboardResponseDto> = {},
): BatteryDashboardResponseDto {
  return {
    schemaVersion: 1,
    collectedAt: timestamp,
    stale: false,
    batteries: [
      {
        id: 'BAT0',
        label: 'Internal battery',
        state: 'discharging',
        updatedAt: timestamp,
        metrics: {
          percentage: available(63),
          energyNowWh: available(35.9, 'sysfs'),
          energyFullWh: available(57.1, 'sysfs'),
          energyDesignWh: available(60, 'sysfs'),
          powerWatts: available(-8.4),
          voltageVolts: available(11.48, 'sysfs'),
          currentAmps: available(-0.73, 'sysfs'),
          temperatureCelsius: available(32.4, 'sysfs'),
          timeRemainingMinutes: available(256, 'derived'),
          cycleCount: available(184, 'sysfs'),
        },
      },
    ],
    ...overrides,
  };
}

describe('normalizeBatteryDashboardResponse', () => {
  it('maps a valid response while preserving provider provenance', () => {
    const dashboard = normalizeBatteryDashboardResponse(response());

    expect(dashboard.batteries).toHaveLength(1);
    expect(dashboard.batteries[0]?.powerWatts).toEqual(available(-8.4));
    expect(dashboard.batteries[0]?.energyNowWh.source).toBe('sysfs');
    expect(dashboard.aggregate.percentage.value).toBe(63);
    expect(dashboard.selectedSnapshot?.kind).toBe('aggregate');
  });

  it('keeps missing readings unavailable instead of converting them to zero', () => {
    const source = response();
    const battery = source.batteries[0];
    if (!battery) throw new Error('test fixture is missing BAT0');
    const dashboard = normalizeBatteryDashboardResponse({
      ...source,
      batteries: [
        {
          ...battery,
          metrics: { ...battery.metrics, temperatureCelsius: unavailable() },
        },
      ],
    });

    expect(dashboard.batteries[0]?.temperatureCelsius).toEqual(unavailable());
    expect(dashboard.batteries[0]?.temperatureCelsius.value).not.toBe(0);
  });

  it('marks fresh readings stale while retaining unknown states and missing metrics', () => {
    const source = response({ stale: true });
    const battery = source.batteries[0];
    if (!battery) throw new Error('test fixture is missing BAT0');
    const dashboard = normalizeBatteryDashboardResponse({
      ...source,
      batteries: [
        {
          ...battery,
          state: 'unknown',
          metrics: { ...battery.metrics, cycleCount: unavailable() },
        },
      ],
    });

    expect(dashboard.stale).toBe(true);
    expect(dashboard.batteries[0]?.state).toBe('unknown');
    expect(dashboard.batteries[0]?.percentage.availability).toBe('stale');
    expect(dashboard.batteries[0]?.cycleCount).toEqual(unavailable());
  });

  it('retains each physical battery and creates a conservative aggregate', () => {
    const first = response().batteries[0];
    if (!first) throw new Error('test fixture is missing BAT0');
    const second = {
      ...first,
      id: 'BAT1',
      label: 'Slice battery',
      state: 'charging' as const,
      metrics: {
        ...first.metrics,
        percentage: available(42, 'sysfs'),
        energyNowWh: available(9.2, 'sysfs'),
        energyFullWh: available(21.9, 'sysfs'),
      },
    };
    const dashboard = normalizeBatteryDashboardResponse(
      response({ batteries: [first, second] }),
    );

    expect(dashboard.batteries.map(({ id }) => id)).toEqual(['BAT0', 'BAT1']);
    expect(dashboard.aggregate.batteryCount).toBe(2);
    expect(dashboard.aggregate.state).toBe('mixed');
    expect(dashboard.aggregate.voltageVolts.availability).toBe('unavailable');
  });

  it('returns an explicit empty dashboard when no batteries are reported', async () => {
    const client = createFixtureBatteryDashboardClient(response({ batteries: [] }));
    const dashboard = await client.getDashboard();

    expect(dashboard.batteries).toEqual([]);
    expect(dashboard.aggregate.batteryCount).toBe(0);
    expect(dashboard.selectedSnapshot).toBeNull();
    expect(dashboard.aggregate.percentage.availability).toBe('unavailable');
  });
});
