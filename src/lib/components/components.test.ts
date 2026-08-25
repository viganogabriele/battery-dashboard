import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import BatterySelector from './BatterySelector.svelte';
import BatteryStateBadge from './BatteryStateBadge.svelte';
import EmptyState from './EmptyState.svelte';
import ExecutionContextNotice from './ExecutionContextNotice.svelte';
import MetricCard from './MetricCard.svelte';
import RecorderSettings from './RecorderSettings.svelte';
import RecentHistoryChart from './RecentHistoryChart.svelte';
import CalendarHistoryView from './CalendarHistoryView.svelte';
import ExportControls from './ExportControls.svelte';
import HealthView from './HealthView.svelte';
import SessionsView from './SessionsView.svelte';

describe('battery dashboard presentation components', () => {
  it('marks an unavailable stale metric honestly', () => {
    render(MetricCard, {
      label: 'Battery temperature',
      value: null,
      source: 'unavailable',
      stale: true,
    });

    expect(screen.getByLabelText('Battery temperature').textContent).toContain(
      'Unavailable',
    );
    expect(screen.getByText('May be outdated')).toBeTruthy();
  });

  it('uses a descriptive label for a battery state', () => {
    render(BatteryStateBadge, { state: 'discharging' });

    expect(screen.getByLabelText('Battery state: On battery')).toBeTruthy();
  });

  it('reports a selected battery through its callback', async () => {
    const onSelect = vi.fn();
    render(BatterySelector, {
      selectedId: 'all-batteries',
      batteries: [
        { id: 'all-batteries', label: 'All batteries' },
        { id: 'BAT0', label: 'Main battery (BAT0)' },
      ],
      onSelect,
    });

    await fireEvent.change(screen.getByLabelText('Battery'), {
      target: { value: 'BAT0' },
    });

    expect(onSelect).toHaveBeenCalledWith('BAT0');
  });

  it('exposes a concise empty-state explanation', () => {
    render(EmptyState, {
      title: 'No battery detected',
      message: 'Connect a supported battery to see readings.',
    });

    expect(screen.getByRole('heading', { name: 'No battery detected' })).toBeTruthy();
  });

  it('discloses when the interface contains simulated data', () => {
    render(ExecutionContextNotice, { executionContext: 'simulated-preview' });

    expect(
      screen.getByRole('heading', { name: 'Simulated battery data' }),
    ).toBeTruthy();
    expect(
      screen.getByText(
        'This screen uses sample readings. It does not read or store system battery data.',
      ),
    ).toBeTruthy();
  });

  it('can describe the desktop context without detecting a runtime', () => {
    render(ExecutionContextNotice, { executionContext: 'native-desktop' });

    expect(screen.getByText('Desktop mode')).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Native desktop window' })).toBeTruthy();
  });

  it('states that recorder collection is opt-in and can enable it', async () => {
    const client = {
      getStatus: vi.fn().mockResolvedValue({
        state: 'disabled' as const,
        lastRecordedAt: null,
        error: null,
      }),
      setEnabled: vi.fn().mockResolvedValue({
        state: 'enabled' as const,
        lastRecordedAt: null,
        error: null,
      }),
    };
    render(RecorderSettings, {
      client,
      initialStatus: { state: 'disabled', lastRecordedAt: null, error: null },
    });

    expect(screen.getByText(/Recording is opt-in and stays local/)).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Enable recording' }));

    expect(client.setEnabled).toHaveBeenCalledWith(true);
    expect(screen.getByText('Recording is active')).toBeTruthy();
  });

  it('does not offer recorder controls on an unsupported system', () => {
    render(RecorderSettings, {
      client: {
        getStatus: vi.fn().mockResolvedValue({ state: 'unsupported' }),
        setEnabled: vi.fn(),
      },
      initialStatus: { state: 'unsupported', lastRecordedAt: null, error: null },
    });

    expect(screen.getByText('Not supported on this system')).toBeTruthy();
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('keeps history gaps visible and never presents them as a continuous reading', () => {
    render(RecentHistoryChart, {
      recorderState: 'enabled',
      points: [
        {
          timestamp: '2026-01-01T09:00:00Z',
          percentage: 70,
          state: 'discharging',
          persisted: true,
        },
        {
          timestamp: '2026-01-01T10:00:00Z',
          percentage: 68,
          state: 'discharging',
          persisted: true,
        },
      ],
      gaps: [
        {
          start: '2026-01-01T09:10:00Z',
          end: '2026-01-01T09:50:00Z',
          reason: 'computer was suspended',
        },
      ],
    });

    expect(screen.getByText(/Gaps in this view/)).toBeTruthy();
    expect(screen.getAllByText(/computer was suspended/)).toHaveLength(2);
    expect(document.querySelectorAll('.recent-history__line')).toHaveLength(0);
  });

  it('explains known gap reasons in calm, specific terms instead of a generic alarm', () => {
    render(RecentHistoryChart, {
      recorderState: 'enabled',
      points: [
        {
          timestamp: '2026-01-01T09:00:00Z',
          percentage: 70,
          state: 'discharging',
          persisted: true,
        },
        {
          timestamp: '2026-01-01T10:00:00Z',
          percentage: 68,
          state: 'discharging',
          persisted: true,
        },
      ],
      gaps: [
        {
          start: '2026-01-01T09:10:00Z',
          end: '2026-01-01T09:20:00Z',
          reason: 'rebooted',
        },
        {
          start: '2026-01-01T09:30:00Z',
          end: '2026-01-01T09:50:00Z',
          reason: 'missing samples',
        },
      ],
    });

    expect(screen.queryByText(/History has gaps\.$/)).toBeNull();
    expect(screen.getAllByText(/the computer restarted here/).length).toBeGreaterThan(
      0,
    );
    expect(
      screen.getAllByText(
        /no samples for a while, likely asleep or recording was paused/,
      ).length,
    ).toBeGreaterThan(0);
  });

  it('reports a chosen range and only renders explicitly supplied summary values', async () => {
    const onRangeChange = vi.fn();
    render(RecentHistoryChart, {
      recorderState: 'enabled',
      selectedRange: 24,
      onRangeChange,
      points: [
        {
          timestamp: '2026-01-01T09:00:00Z',
          percentage: 70,
          state: 'charging',
          persisted: false,
        },
      ],
      summary: { minimumPercentage: 65, observedEnergyWh: 3.4 },
    });

    await fireEvent.click(screen.getByRole('button', { name: '6h' }));

    expect(onRangeChange).toHaveBeenCalledWith(6);
    expect(screen.getByText('65%')).toBeTruthy();
    expect(screen.getByText('3.4 Wh')).toBeTruthy();
    expect(screen.queryByText('Average')).toBeNull();
    expect(screen.getByText('Transient live readings are shown.')).toBeTruthy();
  });

  it('offers multi-day range buttons and reports their hour values', async () => {
    const onRangeChange = vi.fn();
    render(RecentHistoryChart, {
      recorderState: 'enabled',
      selectedRange: 24,
      onRangeChange,
      points: [
        {
          timestamp: '2026-01-01T09:00:00Z',
          percentage: 70,
          state: 'charging',
          persisted: true,
        },
      ],
    });

    expect(screen.getByRole('button', { name: '3d' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '7d' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '30d' })).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: '7d' }));
    expect(onRangeChange).toHaveBeenCalledWith(168);

    await fireEvent.click(screen.getByRole('button', { name: '30d' }));
    expect(onRangeChange).toHaveBeenCalledWith(720);
  });

  it('labels the chart x-axis with date context once the range spans multiple days', () => {
    render(RecentHistoryChart, {
      recorderState: 'enabled',
      selectedRange: 168,
      points: [
        {
          timestamp: '2026-01-01T09:00:00Z',
          percentage: 70,
          state: 'discharging',
          persisted: true,
        },
        {
          timestamp: '2026-01-07T09:00:00Z',
          percentage: 60,
          state: 'discharging',
          persisted: true,
        },
      ],
    });

    const axisText = Array.from(document.querySelectorAll('svg text'))
      .map((node) => node.textContent ?? '')
      .join(' ');
    expect(axisText).toMatch(/Jan/);
  });

  it('shows when the recorded minimum and maximum actually happened', () => {
    render(RecentHistoryChart, {
      recorderState: 'enabled',
      points: [
        {
          timestamp: '2026-01-01T09:00:00Z',
          percentage: 70,
          state: 'discharging',
          persisted: true,
        },
      ],
      summary: {
        minimumPercentage: 15,
        minimumPercentageAt: '2026-01-01T00:34:00Z',
        maximumPercentage: 99,
        maximumPercentageAt: '2026-01-01T14:43:00Z',
        averagePercentage: 66,
        averagePercentageAt: '2026-01-01T16:50:00Z',
      },
    });

    expect(screen.getByText('15%')).toBeTruthy();
    expect(screen.getByText('99%')).toBeTruthy();
    expect(screen.getAllByText(/^at /)).toHaveLength(2);
    expect(screen.getByText(/^as of /)).toBeTruthy();
  });

  it('omits the recorded-time annotation when no timestamp was supplied', () => {
    render(RecentHistoryChart, {
      recorderState: 'enabled',
      points: [
        {
          timestamp: '2026-01-01T09:00:00Z',
          percentage: 70,
          state: 'discharging',
          persisted: true,
        },
      ],
      summary: { minimumPercentage: 65 },
    });

    expect(screen.getByText('65%')).toBeTruthy();
    expect(screen.queryByText(/^at /)).toBeNull();
  });

  it('uses a labelled observed range so small real charge changes stay visible', () => {
    render(RecentHistoryChart, {
      recorderState: 'enabled',
      points: [
        {
          timestamp: '2026-01-01T09:00:00Z',
          percentage: 70,
          state: 'discharging',
          persisted: true,
        },
        {
          timestamp: '2026-01-01T09:05:00Z',
          percentage: 72,
          state: 'discharging',
          persisted: true,
        },
      ],
    });

    expect(screen.getByText('68%')).toBeTruthy();
    expect(screen.getByText('74%')).toBeTruthy();
    expect(document.querySelectorAll('.recent-history__line')).toHaveLength(1);
  });

  it('explains why there is no persistent history when recording is disabled', () => {
    render(RecentHistoryChart, { points: [], recorderState: 'disabled' });

    expect(screen.getByText(/Enable recording to collect local readings/)).toBeTruthy();
  });

  it('shows incomplete session reasons and reports filter changes', async () => {
    const onStateChange = vi.fn();
    render(SessionsView, {
      sessions: [
        {
          id: 's-1',
          batteryId: 'BAT0',
          state: 'discharging',
          startedAt: '2026-01-01T09:00:00Z',
          endedAt: '2026-01-01T10:00:00Z',
          completeness: 'incomplete',
          gapReason: 'computer suspended',
          durationMinutes: 60,
        },
      ],
      onStateChange,
    });

    expect(screen.getByText('Incomplete')).toBeTruthy();
    expect(screen.getByText('Interrupted: computer suspended.')).toBeTruthy();
    await fireEvent.change(screen.getByLabelText('State'), {
      target: { value: 'charging' },
    });
    expect(onStateChange).toHaveBeenCalledWith('charging');
  });

  it('turns complete recorded runs into explicit battery-duration answers', () => {
    render(SessionsView, {
      sessions: [
        {
          id: 'discharge-from-full',
          batteryId: 'BAT0',
          state: 'discharging',
          startedAt: '2026-01-01T09:00:00Z',
          endedAt: '2026-01-01T13:00:00Z',
          completeness: 'complete',
          durationMinutes: 240,
          startPercentage: 100,
          endPercentage: 12,
        },
        {
          id: 'charge-to-full',
          batteryId: 'BAT0',
          state: 'charging',
          startedAt: '2026-01-01T14:00:00Z',
          endedAt: '2026-01-01T16:00:00Z',
          completeness: 'complete',
          durationMinutes: 120,
          startPercentage: 22,
          endPercentage: 100,
        },
      ],
    });

    expect(screen.getByText('What your recorded runs show')).toBeTruthy();
    expect(screen.getAllByText('4h 0m')).toHaveLength(3);
    expect(screen.getAllByText('2h 0m')).toHaveLength(2);
    expect(screen.getAllByText('100% → 12%')).toHaveLength(2);
    expect(screen.getAllByText('22% → 100%')).toHaveLength(1);
    // Each "observed answer" summary card names the calendar day it happened,
    // not just a duration and a percentage range with no date context.
    expect(screen.getAllByText(/Thu, Jan 1, 2026/).length).toBeGreaterThan(0);
  });

  it('keeps unavailable calendar values unavailable and changes aggregation', async () => {
    const onAggregationChange = vi.fn();
    render(CalendarHistoryView, {
      periods: [
        {
          id: '2026-01-01',
          label: '1 Jan 2026',
          observedSamples: null,
          observedEnergyWh: null,
        },
      ],
      onAggregationChange,
    });

    expect(screen.getAllByText('—')).toHaveLength(5);
    await fireEvent.click(screen.getByRole('button', { name: 'weekly' }));
    expect(onAggregationChange).toHaveBeenCalledWith('weekly');
  });

  it('labels explicitly recorded calendar duration without estimating it', () => {
    render(CalendarHistoryView, {
      periods: [
        {
          id: '2026-01-01',
          label: '1 Jan 2026',
          observedSamples: null,
          recordedDurationSeconds: 5_400,
        },
      ],
    });

    expect(screen.getByText('Recorded time')).toBeTruthy();
    expect(screen.getByText('1h 30m')).toBeTruthy();
  });

  it('states when session and calendar history are unavailable', () => {
    render(SessionsView, {
      unsupportedReason: 'This recorder does not provide sessions.',
    });
    render(CalendarHistoryView, {
      unsupportedReason: 'This recorder does not provide calendar summaries.',
    });

    expect(screen.getByText(/Session history is unavailable/)).toBeTruthy();
    expect(screen.getByText(/Calendar history is unavailable/)).toBeTruthy();
  });

  it('shows calculable health, a supported cycle count, and a stable capacity trend', () => {
    render(HealthView, {
      currentFullCapacityWh: 45,
      designCapacityWh: 50,
      healthPercentage: 90,
      healthRecordedAt: '2026-02-01T09:00:00Z',
      hardwareCycleCount: 120,
      trend: 'stable',
      capacityHistory: [
        { timestamp: '2026-01-01T09:00:00Z', fullCapacityWh: 45.2 },
        { timestamp: '2026-02-01T09:00:00Z', fullCapacityWh: 45 },
      ],
    });

    expect(screen.getByText('90.0%')).toBeTruthy();
    expect(screen.getByText('120')).toBeTruthy();
    expect(screen.getByText('Stable capacity')).toBeTruthy();
    expect(
      screen.getByRole('img', { name: /Capacity history with 2 recorded readings/ }),
    ).toBeTruthy();
    // The health percentage always comes from the backend's same-sample pair,
    // never from dividing two independently-latest capacity readings.
    expect(screen.getByText(/currently holds about 45\.0 Wh/)).toBeTruthy();
  });

  it('never derives health from independently-latest capacity readings', () => {
    // currentFullCapacityWh and designCapacityWh alone would divide to 90%,
    // but no healthPercentage was supplied (the two capacities were not
    // observed together), so the health metric must stay honestly unavailable
    // rather than presenting an unearned ratio.
    render(HealthView, {
      currentFullCapacityWh: 45,
      designCapacityWh: 50,
      trend: 'insufficient',
    });

    expect(screen.queryByText('90.0%')).toBeNull();
  });

  it('keeps unavailable health and unsupported cycles distinct, with explicit inconclusive states', () => {
    render(HealthView, { trend: 'noisy' });
    render(HealthView, { id: 'health-insufficient', trend: 'insufficient' });
    render(HealthView, { id: 'health-degrading', trend: 'degrading' });

    expect(screen.getAllByText('Unavailable')).toHaveLength(9);
    expect(screen.getAllByText('Not supported by this battery')).toHaveLength(3);
    expect(screen.getByText('Trend is noisy')).toBeTruthy();
    expect(screen.getByText('Insufficient history')).toBeTruthy();
    expect(screen.getByText('Capacity declining')).toBeTruthy();
    expect(
      screen.getAllByText('No recorded capacity history is available.'),
    ).toHaveLength(3);
  });

  it('reports the explicitly selected export type and format only after a user action', async () => {
    const onExport = vi.fn();
    render(ExportControls, { onExport });

    expect(onExport).not.toHaveBeenCalled();
    await fireEvent.change(screen.getByLabelText('Export data type'), {
      target: { value: 'sessions' },
    });
    await fireEvent.change(screen.getByLabelText('Export format'), {
      target: { value: 'json' },
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Export' }));

    expect(onExport).toHaveBeenCalledWith({ dataType: 'sessions', format: 'json' });
    expect(screen.queryByText(/saved to/i)).toBeNull();
  });
});
