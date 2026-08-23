import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import BatterySelector from './BatterySelector.svelte';
import BatteryStateBadge from './BatteryStateBadge.svelte';
import EmptyState from './EmptyState.svelte';
import ExecutionContextNotice from './ExecutionContextNotice.svelte';
import MetricCard from './MetricCard.svelte';
import RecorderSettings from './RecorderSettings.svelte';

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
});
