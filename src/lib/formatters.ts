export function formatRelativeTime(date: Date, now = new Date()): string {
  const seconds = Math.max(0, Math.round((now.getTime() - date.getTime()) / 1000));

  if (seconds < 60) return 'just now';

  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;

  const hours = Math.round(minutes / 60);
  return `${hours} h ago`;
}

export type CalendarBucketPeriod = 'daily' | 'weekly' | 'monthly';

const MONTH_LABEL = new Intl.DateTimeFormat(undefined, {
  year: 'numeric',
  month: 'long',
});
const DAY_LABEL = new Intl.DateTimeFormat(undefined, {
  weekday: 'short',
  year: 'numeric',
  month: 'short',
  day: 'numeric',
});
const WEEK_RANGE_LABEL = new Intl.DateTimeFormat(undefined, {
  month: 'short',
  day: 'numeric',
});

/** Monday of the given ISO 8601 week, as a local calendar date. */
function isoWeekStart(year: number, week: number): Date {
  // Jan 4th always falls in ISO week 1, so anchoring on it and walking back
  // to that week's Monday gives a reliable start for any requested week.
  const jan4 = new Date(year, 0, 4);
  const jan4Weekday = jan4.getDay() || 7; // Sunday (0) becomes 7 for ISO ordering
  const week1Monday = new Date(year, 0, 4 - jan4Weekday + 1);
  return new Date(
    week1Monday.getFullYear(),
    week1Monday.getMonth(),
    week1Monday.getDate() + (week - 1) * 7,
  );
}

/**
 * Turns a raw backend calendar bucket id ("2026-08-23", "2026-W34",
 * "2026-08") into a label that unambiguously states which day, week, or
 * month a table row covers. The backend intentionally emits bare ISO
 * identifiers; humanizing them is a presentation concern.
 */
export function formatCalendarBucket(
  bucket: string,
  period: CalendarBucketPeriod,
): string {
  if (period === 'daily') {
    const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(bucket);
    if (!match) return bucket;
    const [, year, month, day] = match;
    const date = new Date(Number(year), Number(month) - 1, Number(day));
    return Number.isNaN(date.getTime()) ? bucket : DAY_LABEL.format(date);
  }

  if (period === 'weekly') {
    const match = /^(\d{4})-W(\d{2})$/.exec(bucket);
    if (!match) return bucket;
    const [, year, week] = match;
    const start = isoWeekStart(Number(year), Number(week));
    if (Number.isNaN(start.getTime())) return bucket;
    const end = new Date(start.getFullYear(), start.getMonth(), start.getDate() + 6);
    const range =
      start.getMonth() === end.getMonth()
        ? `${WEEK_RANGE_LABEL.format(start)}–${end.getDate()}`
        : `${WEEK_RANGE_LABEL.format(start)}–${WEEK_RANGE_LABEL.format(end)}`;
    return `Week ${Number(week)}, ${year} (${range})`;
  }

  const match = /^(\d{4})-(\d{2})$/.exec(bucket);
  if (!match) return bucket;
  const [, year, month] = match;
  const date = new Date(Number(year), Number(month) - 1, 1);
  return Number.isNaN(date.getTime()) ? bucket : MONTH_LABEL.format(date);
}
