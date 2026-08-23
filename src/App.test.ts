import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import App from './App.svelte';

describe('App', () => {
  it('describes the current foundation state without claiming live data', () => {
    render(App);

    expect(
      screen.getByRole('heading', { level: 1, name: 'Battery Dashboard' }),
    ).toBeTruthy();
    expect(screen.getByText('Setup in progress')).toBeTruthy();
    expect(screen.getByText('Not connected yet')).toBeTruthy();
  });
});
