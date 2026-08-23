import {
  aggregateBatteries,
  availableMetric,
  staleMetric,
  unavailableMetric,
  type BatteryChartPoint,
  type BatterySnapshot,
  type DashboardScenario,
  type DashboardScenarioCatalog,
} from '../domain/battery';

const currentTimestamp = '2026-08-23T12:00:00.000Z';
const suspendedTimestamp = '2026-08-23T09:43:00.000Z';

function series(
  startPercentage: number,
  endPercentage: number,
  state: BatterySnapshot['state'],
  powerWatts: number | null,
): BatteryChartPoint[] {
  return Array.from({ length: 7 }, (_, index) => ({
    timestamp: `2026-08-23T${String(6 + index).padStart(2, '0')}:00:00.000Z`,
    percentage: startPercentage + ((endPercentage - startPercentage) * index) / 6,
    powerWatts,
    state,
  }));
}

function battery(
  id: string,
  values: {
    label: string;
    state: BatterySnapshot['state'];
    percentage: number;
    energyNowWh: number;
    energyFullWh: number;
    energyDesignWh: number;
    powerWatts: number;
    voltageVolts: number;
    currentAmps: number;
    temperatureCelsius: number | null;
    timeRemainingMinutes: number | null;
    cycleCount: number | null;
    updatedAt?: string;
    stale?: boolean;
  },
): BatterySnapshot {
  const timestamp = values.updatedAt ?? currentTimestamp;
  const metric = values.stale ? staleMetric : availableMetric;

  return {
    kind: 'battery',
    id,
    label: values.label,
    state: values.state,
    percentage: metric(values.percentage, timestamp),
    energyNowWh: metric(values.energyNowWh, timestamp),
    energyFullWh: metric(values.energyFullWh, timestamp),
    energyDesignWh: metric(values.energyDesignWh, timestamp),
    powerWatts: metric(values.powerWatts, timestamp),
    voltageVolts: metric(values.voltageVolts, timestamp),
    currentAmps: metric(values.currentAmps, timestamp),
    temperatureCelsius:
      values.temperatureCelsius === null
        ? unavailableMetric()
        : metric(values.temperatureCelsius, timestamp),
    timeRemainingMinutes:
      values.timeRemainingMinutes === null
        ? unavailableMetric()
        : metric(values.timeRemainingMinutes, timestamp),
    cycleCount:
      values.cycleCount === null
        ? unavailableMetric()
        : metric(values.cycleCount, timestamp),
    updatedAt: timestamp,
  };
}

const normalBat0 = battery('BAT0', {
  label: 'Internal battery',
  state: 'discharging',
  percentage: 63,
  energyNowWh: 35.9,
  energyFullWh: 57.1,
  energyDesignWh: 60,
  powerWatts: -8.4,
  voltageVolts: 11.48,
  currentAmps: -0.73,
  temperatureCelsius: 32.4,
  timeRemainingMinutes: 256,
  cycleCount: 184,
});

const mixedBat0 = battery('BAT0', {
  label: 'Main battery',
  state: 'discharging',
  percentage: 71,
  energyNowWh: 33.4,
  energyFullWh: 47.1,
  energyDesignWh: 50.2,
  powerWatts: -6.2,
  voltageVolts: 11.36,
  currentAmps: -0.55,
  temperatureCelsius: 31.8,
  timeRemainingMinutes: 323,
  cycleCount: 241,
});

const mixedBat1 = battery('BAT1', {
  label: 'Slice battery',
  state: 'charging',
  percentage: 42,
  energyNowWh: 9.2,
  energyFullWh: 21.9,
  energyDesignWh: 23.5,
  powerWatts: 4.1,
  voltageVolts: 11.19,
  currentAmps: 0.37,
  temperatureCelsius: 29.7,
  timeRemainingMinutes: 186,
  cycleCount: 116,
});

const chargingBat0 = battery('BAT0', {
  label: 'Internal battery',
  state: 'charging',
  percentage: 48,
  energyNowWh: 27.5,
  energyFullWh: 57.1,
  energyDesignWh: 60,
  powerWatts: 18.7,
  voltageVolts: 11.66,
  currentAmps: 1.6,
  temperatureCelsius: 34.1,
  timeRemainingMinutes: 95,
  cycleCount: 184,
});

const incompleteBat0: BatterySnapshot = {
  ...battery('BAT0', {
    label: 'Internal battery',
    state: 'discharging',
    percentage: 54,
    energyNowWh: 29.8,
    energyFullWh: 55.2,
    energyDesignWh: 60,
    powerWatts: -7.1,
    voltageVolts: 11.31,
    currentAmps: -0.63,
    temperatureCelsius: null,
    timeRemainingMinutes: null,
    cycleCount: null,
  }),
  currentAmps: unavailableMetric(),
};

const staleBat0 = battery('BAT0', {
  label: 'Internal battery',
  state: 'discharging',
  percentage: 58,
  energyNowWh: 33.1,
  energyFullWh: 57.1,
  energyDesignWh: 60,
  powerWatts: -7.8,
  voltageVolts: 11.42,
  currentAmps: -0.68,
  temperatureCelsius: 31.9,
  timeRemainingMinutes: 255,
  cycleCount: 184,
  updatedAt: suspendedTimestamp,
  stale: true,
});

const scenarios: DashboardScenario[] = [
  {
    id: 'single-discharging',
    name: 'Single battery · discharging',
    description: 'A normal BAT0 discharge with complete telemetry.',
    batteries: [normalBat0],
    aggregate: aggregateBatteries([normalBat0]),
    selectedSnapshot: normalBat0,
    chart: series(87, 63, 'discharging', -8.4),
  },
  {
    id: 'multiple-mixed',
    name: 'Two batteries · mixed state',
    description:
      'BAT0 is discharging while BAT1 is charging; the all-batteries view stays explicit.',
    batteries: [mixedBat0, mixedBat1],
    aggregate: aggregateBatteries([mixedBat0, mixedBat1]),
    selectedSnapshot: aggregateBatteries([mixedBat0, mixedBat1]),
    chart: series(69, 62, 'discharging', -2.1),
  },
  {
    id: 'charging',
    name: 'Single battery · charging',
    description: 'A normal BAT0 charging session with a hardware-provided estimate.',
    batteries: [chargingBat0],
    aggregate: aggregateBatteries([chargingBat0]),
    selectedSnapshot: chargingBat0,
    chart: series(19, 48, 'charging', 18.7),
  },
  {
    id: 'incomplete-telemetry',
    name: 'Incomplete hardware telemetry',
    description: 'The battery works, but its firmware does not expose every metric.',
    batteries: [incompleteBat0],
    aggregate: aggregateBatteries([incompleteBat0]),
    selectedSnapshot: incompleteBat0,
    chart: series(69, 54, 'discharging', -7.1),
  },
  {
    id: 'stale-after-suspend',
    name: 'Stale after suspend',
    description:
      'The last sample predates a suspend; values are deliberately marked stale.',
    batteries: [staleBat0],
    aggregate: aggregateBatteries([staleBat0]),
    selectedSnapshot: staleBat0,
    chart: [
      ...series(84, 58, 'discharging', -7.8),
      {
        timestamp: currentTimestamp,
        percentage: null,
        powerWatts: null,
        state: 'unknown',
      },
    ],
  },
  {
    id: 'no-battery',
    name: 'No battery detected',
    description: 'Desktop systems and unsupported hardware show an honest empty state.',
    batteries: [],
    aggregate: aggregateBatteries([]),
    selectedSnapshot: null,
    chart: [],
  },
];

export const dashboardScenarioCatalog: DashboardScenarioCatalog = {
  defaultScenarioId: 'single-discharging',
  scenarios,
};

export function findDashboardScenario(id: string): DashboardScenario | undefined {
  return dashboardScenarioCatalog.scenarios.find((scenario) => scenario.id === id);
}
