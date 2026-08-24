<script lang="ts">
  export type HistoryRangeHours = 2 | 6 | 12 | 24;

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
    maximumPercentage?: number | null;
    averagePercentage?: number | null;
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

  const ranges: HistoryRangeHours[] = [2, 6, 12, 24];
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

  function gapDescription(gap: RecentHistoryGap): string {
    return gap.reason
      ? `Missing readings: ${gap.reason}`
      : 'Missing readings in this interval';
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

  let segments = $derived.by(() => {
    const result: PathSegment[] = [];

    for (let index = 1; index < validPoints.length; index += 1) {
      const previous = validPoints[index - 1];
      const current = validPoints[index];
      if (hasGapBetween(previous, current)) continue;

      result.push({
        d: `M ${x(previous).toFixed(2)} ${y(previous.percentage).toFixed(2)} L ${x(current).toFixed(2)} ${y(current.percentage).toFixed(2)}`,
        state: current.state,
      });
    }

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
          onclick={() => onRangeChange(range)}>{range}h</button
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
          >Recorded coverage: {Math.floor(coverageSeconds / 60)} minutes ({coveragePercentage}%
          of this view).</strong
        >
        {#if hasUnrecordedPrefix}
          Recording began {formatDateTime(
            firstPersistedTimestamp ?? rangeEndTimestamp,
          )}; the earlier part of this {selectedRange}-hour view has no recorded
          samples.
        {/if}
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
        <strong>History has gaps.</strong>
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
      {#each visibleGaps as gap (`${gap.start}-${gap.end}`)}
        <rect
          class="recent-history__gap-region"
          x={xTimestamp(gap.start)}
          y={padding.top}
          width={Math.max(3, xTimestamp(gap.end) - xTimestamp(gap.start))}
          height={height - padding.top - padding.bottom}
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
        >{formatTime(firstTimestamp)}</text
      >
      <text x={width - padding.right} y={height - 8} text-anchor="end"
        >{formatTime(lastTimestamp)}</text
      >
      {#each segments as segment, index (`${segment.d}-${index}`)}
        <path
          class:recent-history__line--charging={segment.state === 'charging'}
          class:recent-history__line--discharging={segment.state === 'discharging'}
          class="recent-history__line"
          d={segment.d}
        />
      {/each}
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
          <dd>{formatPercentage(summary.minimumPercentage)}</dd>
        </div>{/if}
      {#if isProvided(summary.maximumPercentage)}<div>
          <dt>Maximum</dt>
          <dd>{formatPercentage(summary.maximumPercentage)}</dd>
        </div>{/if}
      {#if isProvided(summary.averagePercentage)}<div>
          <dt>Average</dt>
          <dd>{formatPercentage(summary.averagePercentage)}</dd>
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
    fill: color-mix(in srgb, var(--color-warning), transparent 78%);
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
