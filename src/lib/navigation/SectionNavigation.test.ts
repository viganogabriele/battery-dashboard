import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import SectionNavigation from './SectionNavigation.svelte';
import { productSections } from './sections';

describe('SectionNavigation', () => {
  it('renders every product section and identifies the selected section', () => {
    render(SectionNavigation, { selectedSection: 'history' });

    expect(screen.getByRole('navigation', { name: 'Primary navigation' })).toBeTruthy();

    for (const section of productSections) {
      expect(
        screen.getByRole('button', { name: new RegExp(`^${section.label}:`) }),
      ).toBeTruthy();
    }

    expect(
      screen.getByRole('button', { name: /^History:/ }).getAttribute('aria-current'),
    ).toBe('page');
    expect(
      screen.getByRole('button', { name: /^Dashboard:/ }).getAttribute('aria-current'),
    ).toBeNull();
  });

  it('notifies the parent when a section is selected', async () => {
    const onSelect = vi.fn();
    render(SectionNavigation, { selectedSection: 'dashboard', onSelect });

    await fireEvent.click(screen.getByRole('button', { name: /^Settings:/ }));

    expect(onSelect).toHaveBeenCalledOnce();
    expect(onSelect).toHaveBeenCalledWith('settings');
  });
});
