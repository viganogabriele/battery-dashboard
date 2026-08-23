import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import App from './App.svelte';

describe('App', () => {
  it('refuses to show fixture readings in the browser', () => {
    render(App);

    expect(screen.getByText('Battery Dashboard')).toBeTruthy();
    expect(screen.getByText('Desktop app required')).toBeTruthy();
    expect(screen.getByText('Open the desktop application')).toBeTruthy();
    expect(screen.queryByRole('meter')).toBeNull();
  });

  it('renders calendar history with an explicit empty recorded state', async () => {
    render(App);

    await fireEvent.click(screen.getByRole('button', { name: /^History:/ }));

    expect(
      screen.getByRole('heading', {
        name: 'Recorded battery history',
      }),
    ).toBeTruthy();
    expect(
      screen.getByText('No recorded calendar history matches these filters.'),
    ).toBeTruthy();
  });

  it('shows opt-in recorder settings without pretending browser preview supports them', async () => {
    render(App);

    await fireEvent.click(screen.getByRole('button', { name: /^Settings:/ }));

    expect(screen.getByRole('heading', { name: 'Background recorder' })).toBeTruthy();
    expect(screen.getByText('Not supported on this system')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /Enable recording/ })).toBeNull();
  });
});
