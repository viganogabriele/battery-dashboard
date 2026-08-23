import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import TimeSeriesChart from './TimeSeriesChart.svelte';

describe('TimeSeriesChart', () => {
  it('renders an accessible summary for usable readings', () => {
    render(TimeSeriesChart, {
      id: 'power-chart',
      title: 'Power over time',
      description: 'Simulated readings from the last two hours.',
      unit: ' W',
      points: [
        { timestamp: '2026-08-23T08:00:00Z', value: 8.5 },
        { timestamp: '2026-08-23T09:00:00Z', value: 12.3 },
      ],
    });

    expect(screen.getByRole('img').querySelector('desc')?.textContent).toBe(
      '2 readings. Lowest 8.5 W, highest 12.3 W.',
    );
  });

  it('does not draw a graph for missing readings', () => {
    render(TimeSeriesChart, {
      id: 'empty-power-chart',
      title: 'Power over time',
      description: 'Simulated readings.',
      points: [{ timestamp: '2026-08-23T08:00:00Z', value: null }],
    });

    expect(screen.getByText('No usable readings for this period.')).toBeTruthy();
  });
});
