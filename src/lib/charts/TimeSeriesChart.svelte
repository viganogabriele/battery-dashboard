<script lang="ts">
  export type ChartPoint = { timestamp: Date | string; value: number | null };

  type Props = {
    id: string;
    title: string;
    description: string;
    points: ChartPoint[];
    unit?: string;
    color?: string;
    formatValue?: (value: number) => string;
  };

  let {
    id,
    title,
    description,
    points,
    unit = '',
    color = 'var(--color-accent)',
    formatValue = (value: number) => `${value.toFixed(1)}${unit}`,
  }: Props = $props();

  const width = 640;
  const height = 230;
  const padding = { top: 18, right: 18, bottom: 30, left: 46 };

  function isValidPoint(point: ChartPoint): point is ChartPoint & { value: number } {
    return Number.isFinite(point.value) && Number.isFinite(timestampValue(point));
  }

  function timestampValue(point: ChartPoint): number {
    return new Date(point.timestamp).getTime();
  }

  let validPoints = $derived(points.filter(isValidPoint));
  let values = $derived(validPoints.map((point) => point.value));
  let minimum = $derived(values.length ? Math.min(...values) : 0);
  let maximum = $derived(values.length ? Math.max(...values) : 0);
  let chartMinimum = $derived(minimum === maximum ? minimum - 1 : minimum);
  let chartMaximum = $derived(minimum === maximum ? maximum + 1 : maximum);
  let timestamps = $derived(validPoints.map(timestampValue));
  let firstTimestamp = $derived(timestamps[0] ?? 0);
  let lastTimestamp = $derived(timestamps.at(-1) ?? firstTimestamp + 1);

  function x(point: ChartPoint): number {
    const range = lastTimestamp - firstTimestamp || 1;
    return (
      padding.left +
      ((timestampValue(point) - firstTimestamp) / range) *
        (width - padding.left - padding.right)
    );
  }

  function y(value: number): number {
    return (
      padding.top +
      (1 - (value - chartMinimum) / (chartMaximum - chartMinimum)) *
        (height - padding.top - padding.bottom)
    );
  }

  let path = $derived.by(() => {
    let previousPointWasUsable = false;

    return points
      .map((point) => {
        if (!isValidPoint(point)) {
          previousPointWasUsable = false;
          return '';
        }

        const command = previousPointWasUsable ? 'L' : 'M';
        previousPointWasUsable = true;
        return `${command} ${x(point).toFixed(2)} ${y(point.value).toFixed(2)}`;
      })
      .filter(Boolean)
      .join(' ');
  });
  let gridValues = $derived([
    chartMinimum,
    (chartMinimum + chartMaximum) / 2,
    chartMaximum,
  ]);
  let summary = $derived(
    validPoints.length
      ? `${validPoints.length} readings. Lowest ${formatValue(minimum)}, highest ${formatValue(maximum)}.`
      : 'No readings are available for this period.',
  );
</script>

<section class="time-series-chart" aria-labelledby={`${id}-title`}>
  <header>
    <div>
      <h2 id={`${id}-title`}>{title}</h2>
      <p>{description}</p>
    </div>
    {#if validPoints.length}<span class="time-series-chart__range"
        >{formatValue(minimum)} – {formatValue(maximum)}</span
      >{/if}
  </header>

  {#if validPoints.length}
    <svg
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-labelledby={`${id}-title ${id}-summary`}
    >
      <desc id={`${id}-summary`}>{summary}</desc>
      {#each gridValues as value (value)}
        <line
          x1={padding.left}
          x2={width - padding.right}
          y1={y(value)}
          y2={y(value)}
          class="grid"
        />
        <text x={padding.left - 8} y={y(value) + 4} text-anchor="end"
          >{formatValue(value)}</text
        >
      {/each}
      <path
        d={path}
        fill="none"
        stroke={color}
        stroke-width="3"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  {:else}
    <p class="time-series-chart__empty">No usable readings for this period.</p>
  {/if}
</section>

<style>
  .time-series-chart {
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: 1.25rem;
    background: var(--color-surface);
  }

  header {
    display: flex;
    gap: 1rem;
    justify-content: space-between;
    margin-bottom: 1rem;
  }

  h2,
  p {
    margin: 0;
  }

  h2 {
    font-size: 1.05rem;
  }

  header p,
  .time-series-chart__empty {
    margin-top: 0.3rem;
    color: var(--color-text-secondary);
    font-size: 0.9rem;
  }

  .time-series-chart__range {
    color: var(--color-text-secondary);
    font-size: 0.82rem;
    white-space: nowrap;
  }

  svg {
    display: block;
    width: 100%;
    overflow: visible;
  }

  .grid {
    stroke: var(--color-border-subtle);
    stroke-dasharray: 3 4;
  }

  text {
    fill: var(--color-text-secondary);
    font-size: 11px;
  }

  .time-series-chart__empty {
    display: grid;
    min-height: 12rem;
    place-items: center;
    border: 1px dashed var(--color-border-subtle);
    border-radius: 0.75rem;
  }
</style>
