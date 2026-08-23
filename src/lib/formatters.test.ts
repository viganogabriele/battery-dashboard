import { describe, expect, it } from 'vitest';

import { formatRelativeTime } from './formatters';

describe('formatRelativeTime', () => {
  const now = new Date('2026-08-23T12:00:00Z');

  it('formats a recent timestamp', () => {
    expect(formatRelativeTime(new Date('2026-08-23T11:59:42Z'), now)).toBe('just now');
  });

  it('formats elapsed minutes and hours', () => {
    expect(formatRelativeTime(new Date('2026-08-23T11:45:00Z'), now)).toBe(
      '15 min ago',
    );
    expect(formatRelativeTime(new Date('2026-08-23T09:30:00Z'), now)).toBe('3 h ago');
  });
});
