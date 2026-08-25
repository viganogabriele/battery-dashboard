import { describe, expect, it } from 'vitest';

import { formatCalendarBucket, formatRelativeTime } from './formatters';

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

describe('formatCalendarBucket', () => {
  it('states the full weekday and date for a daily bucket', () => {
    expect(formatCalendarBucket('2026-08-23', 'daily')).toBe('Sun, Aug 23, 2026');
  });

  it('states the ISO week number, year, and the covered date range', () => {
    // 2026-W34 is Mon 17 Aug - Sun 23 Aug 2026.
    expect(formatCalendarBucket('2026-W34', 'weekly')).toBe(
      'Week 34, 2026 (Aug 17–23)',
    );
  });

  it('states the full month and year for a monthly bucket', () => {
    expect(formatCalendarBucket('2026-08', 'monthly')).toBe('August 2026');
  });

  it('falls back to the raw bucket id when it does not match the expected shape', () => {
    expect(formatCalendarBucket('not-a-bucket', 'daily')).toBe('not-a-bucket');
    expect(formatCalendarBucket('not-a-bucket', 'weekly')).toBe('not-a-bucket');
    expect(formatCalendarBucket('not-a-bucket', 'monthly')).toBe('not-a-bucket');
  });
});
