import { describe, expect, it } from 'vitest';

import {
  aggregateBatteries,
  availableMetric,
  type BatterySnapshot,
  unavailableMetric,
} from './battery';

const timestamp = '2026-08-23T10:00:00.000Z';

function battery(overrides: Partial<BatterySnapshot> = {}): BatterySnapshot {
  return {
    kind: 'battery',
    id: 'BAT0',
    label: 'Internal battery',
    state: 'discharging',
    percentage: availableMetric(50, timestamp),
    energyNowWh: availableMetric(25, timestamp),
    energyFullWh: availableMetric(50, timestamp),
    energyDesignWh: availableMetric(60, timestamp),
    powerWatts: availableMetric(-10, timestamp),
    voltageVolts: availableMetric(11.4, timestamp),
    currentAmps: availableMetric(-0.88, timestamp),
    temperatureCelsius: availableMetric(31, timestamp),
    timeRemainingMinutes: availableMetric(150, timestamp),
    cycleCount: availableMetric(100, timestamp),
    updatedAt: timestamp,
    ...overrides,
  };
}

describe('aggregateBatteries', () => {
  it('weights percentage by full capacity and sums compatible energy and power', () => {
    const aggregate = aggregateBatteries([
      battery({
        id: 'BAT0',
        percentage: availableMetric(80, timestamp),
        energyFullWh: availableMetric(40, timestamp),
        powerWatts: availableMetric(-8, timestamp),
      }),
      battery({
        id: 'BAT1',
        percentage: availableMetric(20, timestamp),
        energyFullWh: availableMetric(20, timestamp),
        powerWatts: availableMetric(-4, timestamp),
        state: 'charging',
      }),
    ]);

    expect(aggregate.percentage.value).toBeCloseTo(60);
    expect(aggregate.energyFullWh.value).toBe(60);
    expect(aggregate.powerWatts.value).toBe(-12);
    expect(aggregate.state).toBe('mixed');
    expect(aggregate.voltageVolts.value).toBeNull();
    expect(aggregate.temperatureCelsius.value).toBeNull();
  });

  it('does not substitute a missing device reading with zero', () => {
    const aggregate = aggregateBatteries([
      battery(),
      battery({
        id: 'BAT1',
        powerWatts: unavailableMetric(),
        energyNowWh: unavailableMetric(),
      }),
    ]);

    expect(aggregate.powerWatts).toMatchObject({
      value: null,
      availability: 'unavailable',
    });
    expect(aggregate.energyNowWh).toMatchObject({
      value: null,
      availability: 'unavailable',
    });
  });

  it('keeps individual-only metrics when there is a single battery', () => {
    const source = battery();
    const aggregate = aggregateBatteries([source]);

    expect(aggregate.voltageVolts).toEqual(source.voltageVolts);
    expect(aggregate.timeRemainingMinutes).toEqual(source.timeRemainingMinutes);
  });

  it('represents the absence of batteries explicitly', () => {
    const aggregate = aggregateBatteries([]);

    expect(aggregate.state).toBe('unknown');
    expect(aggregate.batteryCount).toBe(0);
    expect(aggregate.percentage.value).toBeNull();
  });
});
