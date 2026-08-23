import { describe, expect, it } from 'vitest';

import { dashboardScenarioCatalog, findDashboardScenario } from './dashboard-scenarios';

describe('dashboardScenarioCatalog', () => {
  it('contains every Phase 2 state needed by the simulated dashboard', () => {
    expect(dashboardScenarioCatalog.scenarios.map((scenario) => scenario.id)).toEqual([
      'single-discharging',
      'multiple-mixed',
      'charging',
      'incomplete-telemetry',
      'stale-after-suspend',
      'no-battery',
    ]);
  });

  it('keeps missing telemetry explicitly unavailable', () => {
    const scenario = findDashboardScenario('incomplete-telemetry');
    const battery = scenario?.batteries[0];

    expect(battery?.temperatureCelsius.value).toBeNull();
    expect(battery?.timeRemainingMinutes.availability).toBe('unavailable');
    expect(battery?.currentAmps.value).toBeNull();
  });

  it('uses a mixed aggregate only when multiple batteries disagree on state', () => {
    const scenario = findDashboardScenario('multiple-mixed');

    expect(scenario?.aggregate.state).toBe('mixed');
    expect(scenario?.aggregate.percentage.value).not.toBeNull();
    expect(scenario?.aggregate.temperatureCelsius.value).toBeNull();
  });

  it('marks stale values after suspend and never pretends a sample exists now', () => {
    const scenario = findDashboardScenario('stale-after-suspend');

    expect(scenario?.batteries[0]?.percentage.availability).toBe('stale');
    expect(scenario?.chart.at(-1)).toMatchObject({
      percentage: null,
      powerWatts: null,
      state: 'unknown',
    });
  });

  it('has an explicit empty state for systems without a battery', () => {
    const scenario = findDashboardScenario('no-battery');

    expect(scenario?.batteries).toHaveLength(0);
    expect(scenario?.selectedSnapshot).toBeNull();
    expect(scenario?.aggregate.percentage.value).toBeNull();
  });
});
