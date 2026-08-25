<script lang="ts">
  /**
   * `72`/`168`/`720` are the 3-day/7-day/30-day views, expressed in hours so
   * the request payload and the backend keep a single unit.
   */
  export type HistoryRangeHours = 2 | 6 | 12 | 24 | 72 | 168 | 720;

  export type RecentHistoryState =
    'charging' | 'discharging' | 'full' | 'idle' | 'unknown';

  export type RecentHistoryPoint = {
    timestamp: Date | string;
    percentage: number | null;
    state: RecentHistoryState;
    /** Stored readings survive app restarts; transient readings only exist in this view. */
    persisted: boolean;
  };

  /** An interval where readings are absent, for example while the computer was suspended. */
  export type RecentHistoryGap = {
    start: Date | string;
    end: Date | string;
    reason?: string;
  };

  /** Values are intentionally rendered only when the data layer explicitly supplies them. */
  export type RecentHistorySummary = {
    minimumPercentage?: number | null;
    /** When the minimum was actually recorded; distinct from the last sample time. */
    minimumPercentageAt?: Date | string | null;
    maximumPercentage?: number | null;
    /** When the maximum was actually recorded. */
    maximumPercentageAt?: Date | string | null;
    averagePercentage?: number | null;
    /** The last persisted sample time the average was computed as of. */
    averagePercentageAt?: Date | string | null;
    observedEnergyWh?: number | null;
  };

  export type RecentHistoryRecorderState =
    'enabled' | 'disabled' | 'unsupported' | 'error';

  type Props = {
    id?: string;
    points: RecentHistoryPoint[];
    gaps?: RecentHistoryGap[];
    summary?: RecentHistorySummary | null;
    loading?: boolean;
    recorderState?: RecentHistoryRecorderState;
    selectedRange?: HistoryRangeHours;
    rangeEnd?: Date | string | null;
    onRangeChange?: (hours: HistoryRangeHours) => void;
  };

  let {
    id = 'recent-history',
    points,
    gaps = [],
    summary = null,
    loading = false,
    recorderState = 'disabled',
    selectedRange = 24,
    rangeEnd = null,
    onRangeChange = () => {},
  }: Props = $props();

  const ranges: HistoryRangeHours[] = [2, 6, 12, 24, 72, 168, 720];

  /** Compact button label: hours below a day, whole days at and beyond it. */
  function rangeLabel(hours: HistoryRangeHours): string {
    return hours < 24 ? `${hours}h` : `${hours / 24}d`;
  }

  /** A recorded-coverage duration that stays legible from minutes to weeks. */
  function formatCoverageDuration(seconds: number): string {
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes} min`;
    const hours = Math.floor(minutes / 60);
    const remainderMinutes = minutes % 60;
    if (hours < 24) {
      return remainderMinutes > 0 ? `${hours}h ${remainderMinutes}m` : `${hours}h`;
    }
    const days = Math.floor(hours / 24);
    const remainderHours = hours % 24;
    return remainderHours > 0 ? `${days}d ${remainderHours}h` : `${days}d`;
  }
  const width = 680;
  const height = 244;
  const padding = { top: 20, right: 20, bottom: 32, left: 42 };

  type ValidPoint = RecentHistoryPoint & { percentage: number; timestampMs: number };
  type PathSegment = { d: string; state: RecentHistoryState };

  function timestampMs(value: Date | string): number {
    return new Date(value).getTime();
  }

  function toValidPoint(point: RecentHistoryPoint): ValidPoint | null {
    const timestamp = timestampMs(point.timestamp);
    if (
      Number.isFinite(timestamp) &&
      Number.isFinite(point.percentage) &&
      point.percentage !== null &&
      point.percentage >= 0 &&
      point.percentage <= 100
    ) {
      return { ...point, percentage: point.percentage, timestampMs: timestamp };
    }

    return null;
  }

  function isProvided(value: number | null | undefined): value is number {
    return value !== null && value !== undefined && Number.isFinite(value);
  }

  function hasGapBetween(from: ValidPoint, to: ValidPoint): boolean {
    return gaps.some((gap) => {
      const start = timestampMs(gap.start);
      const end = timestampMs(gap.end);
      return (
        Number.isFinite(start) &&
        Number.isFinite(end) &&
        from.timestampMs <= start &&
        to.timestampMs >= end
      );
    });
  }

  function formatPercentage(value: number): string {
    return `${Math.round(value)}%`;
  }

  function formatEnergy(value: number): string {
    return `${value.toFixed(1)} Wh`;
  }

  function formatTime(value: number): string {
    return new Intl.DateTimeFormat(undefined, {
      hour: 'numeric',
      minute: '2-digit',
    }).format(new Date(value));
  }

  function formatDateTime(value: number): string {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(value));
  }

  /**
   * A bare hour like "7:07 PM" is ambiguous once a view spans more than a
   * day: "Aug 24, 7:07 PM" keeps the x-axis legible across day boundaries.
   */
  function formatAxisLabel(value: number, range: HistoryRangeHours): string {
    if (range <= 24) return formatTime(value);
    return new Intl.DateTimeFormat(undefined, {
      month: 'short',
      day: 'numeric',
      hour: 'numeric',
      minute: '2-digit',
    }).format(new Date(value));
  }

  /** Renders a recorded instant next to a summary value, or nothing when absent. */
  function formatRecordedTime(value: Date | string | null | undefined): string | null {
    if (value === null || value === undefined) return null;
    const parsed = timestampMs(value);
    return Number.isFinite(parsed) ? formatTime(parsed) : null;
  }

  /**
   * Gaps are expected (sleep, shutdown, or history simply not going back that
   * far yet) so the copy stays factual instead of reading as an alarm. Known
   * backend-derived reasons get a specific, calm explanation; an unrecognized
   * reason string is shown as-is rather than dropped.
   */
  function gapDescription(gap: RecentHistoryGap): string {
    switch (gap.reason) {
      case 'rebooted':
        return 'the computer restarted here';
      case 'missing samples':
        return 'no samples for a while, likely asleep or recording was paused';
      case undefined:
        return 'no samples were recorded here';
      default:
        return gap.reason;
    }
  }

  let validPoints = $derived(
    points
      .map(toValidPoint)
      .filter((point): point is ValidPoint => point !== null)
      .sort((a, b) => a.timestampMs - b.timestampMs),
  );
  let firstTimestamp = $derived(validPoints[0]?.timestampMs ?? 0);
  let lastTimestamp = $derived(validPoints.at(-1)?.timestampMs ?? firstTimestamp + 1);
  let rangeEndTimestamp = $derived.by(() => {
    const parsed = rangeEnd === null ? Number.NaN : timestampMs(rangeEnd);
    return Number.isFinite(parsed)
      ? parsed
      : (validPoints.at(-1)?.timestampMs ?? Date.now());
  });
  let requestedStartTimestamp = $derived(
    rangeEndTimestamp - selectedRange * 60 * 60 * 1_000,
  );
  let persistedPoints = $derived(validPoints.filter((point) => point.persisted));
  let firstPersistedTimestamp = $derived(persistedPoints[0]?.timestampMs ?? null);
  let coverageSeconds = $derived(
    firstPersistedTimestamp === null || persistedPoints.length < 2
      ? 0
      : Math.max(0, rangeEndTimestamp - firstPersistedTimestamp) / 1_000,
  );
  let requestedSeconds = $derived(selectedRange * 60 * 60);
  let coveragePercentage = $derived(
    Math.min(100, Math.round((coverageSeconds / requestedSeconds) * 100)),
  );
  let hasUnrecordedPrefix = $derived(
    firstPersistedTimestamp !== null &&
      firstPersistedTimestamp > requestedStartTimestamp,
  );
  let observedState = $derived(
    persistedPoints.length &&
      persistedPoints.every((point) => point.state === persistedPoints[0]?.state)
      ? persistedPoints[0]?.state
      : null,
  );
  let observedPercentageChange = $derived(
    persistedPoints.length >= 2
      ? persistedPoints.at(-1)!.percentage - persistedPoints[0]!.percentage
      : null,
  );
  let observedPercentageRate = $derived(
    observedState !== 'charging' && observedState !== 'discharging'
      ? null
      : coverageSeconds >= 15 * 60 &&
          gaps.length === 0 &&
          observedPercentageChange !== null
        ? observedPercentageChange / (coverageSeconds / 3_600)
        : null,
  );
  // A full 0–100% axis makes an actual one- or two-percent change appear
  // flat. This is still an honest axis: its labels expose the tight observed
  // range, clamped to the physical battery limits.
  let observedMinimum = $derived(
    validPoints.length ? Math.min(...validPoints.map((point) => point.percentage)) : 0,
  );
  let observedMaximum = $derived(
    validPoints.length
      ? Math.max(...validPoints.map((point) => point.percentage))
      : 100,
  );
  let chartPadding = $derived(
    validPoints.length ? Math.max(2, (observedMaximum - observedMinimum) * 0.2) : 0,
  );
  let chartMinimum = $derived(
    validPoints.length ? Math.max(0, Math.floor(observedMinimum - chartPadding)) : 0,
  );
  let chartMaximum = $derived(
    validPoints.length ? Math.min(100, Math.ceil(observedMaximum + chartPadding)) : 100,
  );
  let gridValues = $derived([
    chartMinimum,
    (chartMinimum + chartMaximum) / 2,
    chartMaximum,
  ]);
  let hasPersistedPoints = $derived(validPoints.some((point) => point.persisted));
  let hasTransientPoints = $derived(validPoints.some((point) => !point.persisted));

  function x(point: ValidPoint): number {
    return xTimestamp(point.timestampMs);
  }

  function xTimestamp(timestamp: number): number {
    const range = lastTimestamp - firstTimestamp || 1;
    return (
      padding.left +
      ((timestamp - firstTimestamp) / range) * (width - padding.left - padding.right)
    );
  }

  function y(value: number): number {
    return (
      padding.top +
      (1 - (value - chartMinimum) / (chartMaximum - chartMinimum)) *
        (height - padding.top - padding.bottom)
    );
  }

  // Consecutive samples are frequently only a fraction of a pixel apart once
  // a multi-hour range is compressed into a fixed-width chart (dozens of
  // readings a minute over many hours). Emitting one <path> per adjacent
  // pair used to look like a continuous line only when each pair happened to
  // be several pixels wide; at sub-pixel spacing every independently
  // rasterized two-point path renders as its own tiny antialiased speck, so
  // the stroke reads as a trail of disconnected dots instead of one line.
  // Building a single multi-point path per uninterrupted run fixes this: the
  // renderer computes one continuous stroke outline for the whole run
  // regardless of how close the individual samples are, while still
  // starting a new path (and thus a real visual break) at every actual gap.
  // A state change without a gap still needs its own color, so the run also
  // breaks there -- but the boundary sample is repeated as the first point
  // of the next run so the two runs still touch with no visible seam.
  let segments = $derived.by(() => {
    const result: PathSegment[] = [];
    let run: ValidPoint[] = [];

    function flushRun(): void {
      if (run.length < 2) return;
      const [head, ...rest] = run;
      const d = [
        `M ${x(head).toFixed(2)} ${y(head.percentage).toFixed(2)}`,
        ...rest.map(
          (point) => `L ${x(point).toFixed(2)} ${y(point.percentage).toFixed(2)}`,
        ),
      ].join(' ');
      result.push({ d, state: run.at(-1)!.state });
    }

    for (const point of validPoints) {
      const previous = run.at(-1);
      if (previous === undefined) {
        run = [point];
        continue;
      }
      if (hasGapBetween(previous, point)) {
        flushRun();
        run = [point];
        continue;
      }
      if (point.state !== previous.state) {
        flushRun();
        run = [previous, point];
        continue;
      }
      run.push(point);
    }
    flushRun();

    return result;
  });

  let visibleGaps = $derived.by(() =>
    gaps
      .map((gap) => ({
        start: timestampMs(gap.start),
        end: timestampMs(gap.end),
        label: gapDescription(gap),
      }))
      .filter(
        (gap) =>
          Number.isFinite(gap.start) && Number.isFinite(gap.end) && gap.end > gap.start,
      ),
  );

  let stateDescription = $derived.by(() => {
    if (loading) return 'Loading recent local readings.';
    if (validPoints.length) {
      if (recorderState === 'disabled' && hasPersistedPoints)
        return 'Stored local readings are shown. Background recording is currently disabled.';
      if (hasPersistedPoints && hasTransientPoints)
        return 'Stored readings and transient live readings are shown.';
      return hasPersistedPoints
        ? 'Stored local readings are shown.'
        : 'Transient live readings are shown.';
    }
    if (recorderState === 'disabled')
      return 'Background recording is disabled. Enable it to build persistent history.';
    if (recorderState === 'unsupported')
      return 'Background recording is unavailable on this system.';
    if (recorderState === 'error')
      return 'The recorder needs attention before it can build history.';
    return 'Recording is active, but no usable readings have been collected for this period.';
  });
</script>

<section class="recent-history" aria-labelledby={`${id}-title`}>
  <header class="recent-history__header">
    <div>
      <p class="recent-history__eyebrow">Recent history</p>
      <h2 id={`${id}-title`}>Battery level over time</h2>
      <p class="recent-history__description">{stateDescription}</p>
    </div>

    <div class="recent-history__ranges" aria-label="History range">
      {#each ranges as range (range)}
        <button
          type="button"
          aria-pressed={selectedRange === range}
          onclick={() => onRangeChange(range)}>{rangeLabel(range)}</button
        >
      {/each}
    </div>
  </header>

  {#if loading}
    <div class="recent-history__placeholder" role="status">Loading local history…</div>
  {:else if validPoints.length}
    {#if persistedPoints.length}
      <p class="recent-history__coverage" role="status">
        <strong
          >{formatCoverageDuration(coverageSeconds)} recorded ({coveragePercentage}% of
          this
          {rangeLabel(selectedRange)} view){#if hasUnrecordedPrefix}, starting {formatDateTime(
              firstPersistedTimestamp ?? rangeEndTimestamp,
            )}{/if}.</strong
        >
        {#if observedPercentageRate !== null}
          Observed {observedState === 'charging' ? 'charge' : 'discharge'}: {Math.abs(
            observedPercentageChange ?? 0,
          ).toFixed(1)}% at {Math.abs(observedPercentageRate).toFixed(1)}%/h.
        {/if}
      </p>
    {/if}
    <div class="recent-history__legend" aria-label="Chart legend">
      <span
        ><i class="recent-history__legend-line recent-history__legend-line--charging"
        ></i>Charging</span
      >
      <span
        ><i class="recent-history__legend-line recent-history__legend-line--discharging"
        ></i>Discharging</span
      >
      <span
        ><i class="recent-history__legend-dot recent-history__legend-dot--persisted"
        ></i>Stored</span
      >
      <span
        ><i class="recent-history__legend-dot recent-history__legend-dot--transient"
        ></i>Transient</span
      >
    </div>

    {#if gaps.length}
      <div class="recent-history__gaps" role="status">
        <strong
          >Gaps in this view (expected after sleep, restarts, or before recording
          started):</strong
        >
        {#each gaps as gap, index (`${timestampMs(gap.start)}-${timestampMs(gap.end)}-${index}`)}
          <span>{gapDescription(gap)}.</span>
        {/each}
      </div>
    {/if}

    <svg
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-labelledby={`${id}-title ${id}-summary`}
    >
      <desc id={`${id}-summary`}>
        {validPoints.length} readings. Lines do not connect across {gaps.length} recorded
        gap{gaps.length === 1 ? '' : 's'}.
      </desc>
      <defs>
        <!-- A wide, honest gap (for example an overnight suspend spanning
             most of the visible window) must stay clearly distinguishable
             without turning into a heavy wall of warning color. A sparse
             diagonal hatch reads as "no data here" at any width, whereas a
             flat translucent fill gets visually denser the wider the gap
             is, which is exactly backwards. -->
        <pattern
          id={`${id}-gap-hatch`}
          width="8"
          height="8"
          patternTransform="rotate(45)"
          patternUnits="userSpaceOnUse"
        >
          <line
            x1="0"
            y1="0"
            x2="0"
            y2="8"
            stroke="var(--color-warning)"
            stroke-width="3"
            stroke-opacity="0.55"
          />
        </pattern>
      </defs>
      {#each visibleGaps as gap (`${gap.start}-${gap.end}`)}
        <rect
          class="recent-history__gap-region"
          x={xTimestamp(gap.start)}
          y={padding.top}
          width={Math.max(3, xTimestamp(gap.end) - xTimestamp(gap.start))}
          height={height - padding.top - padding.bottom}
          fill={`url(#${id}-gap-hatch)`}
        >
          <title>{gap.label}</title>
        </rect>
      {/each}
      {#each gridValues as value (value)}
        <line
          class="recent-history__grid"
          x1={padding.left}
          x2={width - padding.right}
          y1={y(value)}
          y2={y(value)}
        />
        <text x={padding.left - 8} y={y(value) + 4} text-anchor="end">{value}%</text>
      {/each}
      <text x={padding.left} y={height - 8} text-anchor="start"
        >{formatAxisLabel(firstTimestamp, selectedRange)}</text
      >
      <text x={width - padding.right} y={height - 8} text-anchor="end"
        >{formatAxisLabel(lastTimestamp, selectedRange)}</text
      >
      <!-- Point markers are drawn before the line, not after. At the sample
           density a multi-hour recorder history reaches (dozens of readings
           a minute, once compressed into a fixed-width chart), consecutive
           markers sit less than a pixel apart; every later circle's
           background-colored outline (`stroke: var(--color-surface)`) then
           paints a ring over part of the previous circle and over the line
           connecting them, punching a trail of tiny background-colored gaps
           through what should read as one continuous stroke. Painting the
           solid line stroke last covers that woven artifact wherever
           samples are dense, while still leaving each marker's outline
           visible wherever points are sparse enough not to overlap. -->
      {#each validPoints as point (`${point.timestampMs}-${point.persisted}`)}
        <circle
          class:recent-history__point--charging={point.state === 'charging'}
          class:recent-history__point--discharging={point.state === 'discharging'}
          class:recent-history__point--transient={!point.persisted}
          class="recent-history__point"
          cx={x(point)}
          cy={y(point.percentage)}
          r="4"
        >
          <title
            >{`${formatPercentage(point.percentage)} · ${point.state} · ${point.persisted ? 'stored' : 'transient'}`}</title
          >
        </circle>
      {/each}
      {#each segments as segment, index (`${segment.d}-${index}`)}
        <path
          class:recent-history__line--charging={segment.state === 'charging'}
          class:recent-history__line--discharging={segment.state === 'discharging'}
          class="recent-history__line"
          d={segment.d}
        />
      {/each}
    </svg>
  {:else}
    <div class="recent-history__placeholder" role="status">
      {recorderState === 'disabled'
        ? 'No persistent history yet. Enable recording to collect local readings.'
        : recorderState === 'enabled'
          ? 'No usable local readings have been collected for this period.'
          : stateDescription}
    </div>
  {/if}

  {#if summary && (isProvided(summary.minimumPercentage) || isProvided(summary.maximumPercentage) || isProvided(summary.averagePercentage) || isProvided(summary.observedEnergyWh))}
    <dl class="recent-history__summary" aria-label="Observed summary">
      {#if isProvided(summary.minimumPercentage)}<div>
          <dt>Minimum</dt>
          <dd>
            {formatPercentage(summary.minimumPercentage)}
            {#if formatRecordedTime(summary.minimumPercentageAt)}
              <span class="recent-history__summary-time"
                >at {formatRecordedTime(summary.minimumPercentageAt)}</span
              >
            {/if}
          </dd>
        </div>{/if}
      {#if isProvided(summary.maximumPercentage)}<div>
          <dt>Maximum</dt>
          <dd>
            {formatPercentage(summary.maximumPercentage)}
            {#if formatRecordedTime(summary.maximumPercentageAt)}
              <span class="recent-history__summary-time"
                >at {formatRecordedTime(summary.maximumPercentageAt)}</span
              >
            {/if}
          </dd>
        </div>{/if}
      {#if isProvided(summary.averagePercentage)}<div>
          <dt>Average</dt>
          <dd>
            {formatPercentage(summary.averagePercentage)}
            {#if formatRecordedTime(summary.averagePercentageAt)}
              <span class="recent-history__summary-time"
                >as of {formatRecordedTime(summary.averagePercentageAt)}</span
              >
            {/if}
          </dd>
        </div>{/if}
      {#if isProvided(summary.observedEnergyWh)}<div>
          <dt>Observed energy</dt>
          <dd>{formatEnergy(summary.observedEnergyWh)}</dd>
        </div>{/if}
    </dl>
  {/if}
</section>

<style>
  .recent-history {
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: 1.25rem;
    background: var(--color-surface);
  }
  .recent-history__header {
    display: flex;
    gap: 1rem;
    align-items: flex-start;
    justify-content: space-between;
  }
  .recent-history__eyebrow,
  h2,
  p {
    margin: 0;
  }
  .recent-history__eyebrow {
    color: var(--color-accent);
    font-size: 0.72rem;
    font-weight: 750;
    letter-spacing: 0.1em;
    text-transform: uppercase;
  }
  h2 {
    margin-top: 0.18rem;
    font-size: 1.05rem;
  }
  .recent-history__description {
    max-width: 58ch;
    margin-top: 0.4rem;
    color: var(--color-text-secondary);
    font-size: 0.88rem;
    line-height: 1.45;
  }
  .recent-history__ranges {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    gap: 0.35rem;
  }
  .recent-history__ranges button {
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.55rem;
    padding: 0.35rem 0.55rem;
    color: var(--color-text-secondary);
    background: transparent;
    font: inherit;
    font-size: 0.8rem;
    font-weight: 700;
    cursor: pointer;
  }
  .recent-history__ranges button[aria-pressed='true'] {
    border-color: color-mix(in srgb, var(--color-accent), transparent 35%);
    color: var(--color-accent-ink);
    background: var(--color-accent);
  }
  .recent-history__legend {
    display: flex;
    flex-wrap: wrap;
    gap: 0.7rem 1rem;
    margin: 1.1rem 0 0.65rem;
    color: var(--color-text-secondary);
    font-size: 0.76rem;
  }
  .recent-history__coverage {
    margin: 0.85rem 0 0;
    border-left: 3px solid var(--color-status);
    padding: 0.45rem 0.65rem;
    color: var(--color-text-secondary);
    background: color-mix(in srgb, var(--color-status), transparent 92%);
    font-size: 0.78rem;
    line-height: 1.4;
  }
  .recent-history__coverage strong {
    color: var(--color-text-primary);
  }
  .recent-history__legend span {
    display: inline-flex;
    gap: 0.35rem;
    align-items: center;
  }
  .recent-history__legend-line {
    width: 1.1rem;
    height: 3px;
    border-radius: 1rem;
    background: var(--color-status);
  }
  .recent-history__legend-line--charging {
    background: var(--color-accent);
  }
  .recent-history__legend-line--discharging {
    background: var(--color-power);
  }
  .recent-history__legend-dot {
    width: 0.65rem;
    height: 0.65rem;
    border: 2px solid var(--color-text-secondary);
    border-radius: 50%;
    background: var(--color-text-secondary);
  }
  .recent-history__legend-dot--transient {
    background: var(--color-surface);
  }
  .recent-history__gaps {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem 0.6rem;
    margin: 0.5rem 0;
    border-left: 3px solid var(--color-warning);
    padding: 0.55rem 0.7rem;
    color: var(--color-text-secondary);
    background: color-mix(in srgb, var(--color-warning), transparent 91%);
    font-size: 0.8rem;
    line-height: 1.35;
  }
  svg {
    display: block;
    width: 100%;
    margin-top: 0.6rem;
    overflow: visible;
  }
  .recent-history__grid {
    stroke: var(--color-border-subtle);
    stroke-dasharray: 3 4;
  }
  .recent-history__gap-region {
    /* The fill itself is a diagonal hatch pattern (see the <defs> in the
       markup) set via the element's `fill` attribute rather than here: a
       flat translucent fill gets visually denser the wider a gap is, so a
       single large real gap (for example an overnight suspend) reads as an
       overwhelming solid block instead of a clearly-marked "no data" band.
       A sparse hatch stays legible at any width. Do not set `fill` in this
       rule; it would take precedence over the per-element pattern
       reference. */
    stroke: var(--color-warning);
    stroke-width: 1;
    stroke-dasharray: 3 3;
  }
  text {
    fill: var(--color-text-secondary);
    font-size: 11px;
  }
  .recent-history__line {
    fill: none;
    stroke: var(--color-status);
    stroke-width: 3;
    stroke-linecap: round;
  }
  .recent-history__line--charging {
    stroke: var(--color-accent);
  }
  .recent-history__line--discharging {
    stroke: var(--color-power);
  }
  .recent-history__point {
    fill: var(--color-status);
    stroke: var(--color-surface);
    stroke-width: 2;
  }
  .recent-history__point--charging {
    fill: var(--color-accent);
  }
  .recent-history__point--discharging {
    fill: var(--color-power);
  }
  .recent-history__point--transient {
    fill: var(--color-surface);
    stroke: var(--color-status);
  }
  .recent-history__point--transient.recent-history__point--charging {
    stroke: var(--color-accent);
  }
  .recent-history__point--transient.recent-history__point--discharging {
    stroke: var(--color-power);
  }
  .recent-history__placeholder {
    display: grid;
    min-height: 12rem;
    margin-top: 1rem;
    place-items: center;
    border: 1px dashed var(--color-border-subtle);
    border-radius: 0.75rem;
    padding: 1rem;
    color: var(--color-text-secondary);
    text-align: center;
  }
  .recent-history__summary {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(8rem, 1fr));
    gap: 0.6rem;
    margin: 1rem 0 0;
  }
  .recent-history__summary div {
    border-radius: 0.65rem;
    padding: 0.65rem 0.75rem;
    background: var(--color-surface-raised);
  }
  dt {
    color: var(--color-text-secondary);
    font-size: 0.72rem;
  }
  dd {
    margin: 0.15rem 0 0;
    font-size: 0.9rem;
    font-weight: 700;
  }
  .recent-history__summary-time {
    margin-left: 0.3rem;
    color: var(--color-text-secondary);
    font-size: 0.72rem;
    font-weight: 500;
  }
  @media (max-width: 34rem) {
    .recent-history__header {
      flex-direction: column;
    }
    .recent-history__ranges {
      width: 100%;
    }
    .recent-history__ranges button {
      flex: 1;
    }
  }
</style>
