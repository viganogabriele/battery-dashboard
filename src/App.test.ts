import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import App from './App.svelte';

describe('App', () => {
  it('renders the simulated dashboard without claiming live collection', () => {
    render(App);

    expect(screen.getByText('Battery Dashboard')).toBeTruthy();
    expect(
      screen.getByRole('heading', {
        level: 1,
        name: 'Current battery status and recent activity.',
      }),
    ).toBeTruthy();
    expect(
      screen.getByText('Simulated data', { selector: '.preview-badge' }),
    ).toBeTruthy();
    expect(screen.getByText(/No battery data is read or stored/)).toBeTruthy();
    expect(screen.getByRole('meter', { name: 'Battery charge: 63%' })).toBeTruthy();
  });

  it('shows the explicit no-battery state for the matching scenario', async () => {
    render(App);

    await fireEvent.change(screen.getByLabelText('Simulation scenario'), {
      target: { value: 'no-battery' },
    });

    expect(screen.getByRole('heading', { name: 'No battery detected' })).toBeTruthy();
    expect(screen.queryByLabelText('Battery metrics')).toBeNull();
  });

  it('renders active navigation as a planned phase rather than false data', async () => {
    render(App);

    await fireEvent.click(screen.getByRole('button', { name: /^History:/ }));

    expect(
      screen.getByRole('heading', {
        name: 'History arrives in a later phase',
      }),
    ).toBeTruthy();
  });
});
