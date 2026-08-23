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

  it('offers today and yesterday shortcuts for recorded sessions and calendar history', async () => {
    render(App);
    const today = new Date();
    const todayIso = [
      today.getFullYear(),
      String(today.getMonth() + 1).padStart(2, '0'),
      String(today.getDate()).padStart(2, '0'),
    ].join('-');

    await fireEvent.click(screen.getByRole('button', { name: /^Sessions:/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Today' }));
    expect((screen.getByLabelText('From') as HTMLInputElement).value).toBe(todayIso);
    expect((screen.getByLabelText('To') as HTMLInputElement).value).toBe(todayIso);

    await fireEvent.click(screen.getByRole('button', { name: /^History:/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Yesterday' }));
    expect((screen.getByLabelText('From') as HTMLInputElement).value).not.toBe(
      todayIso,
    );
  });
});
