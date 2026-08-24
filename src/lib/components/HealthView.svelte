<script lang="ts">
  export type CapacityHistoryPoint = {
    timestamp: Date | string;
    fullCapacityWh: number | null;
  };

  /** Trend labels are supplied by the conservative history analysis. */
  export type CapacityTrend = 'stable' | 'degrading' | 'noisy' | 'insufficient';

  type Props = {
    id?: string;
    currentFullCapacityWh?: number | null;
    designCapacityWh?: number | null;
    hardwareCycleCount?: number | null;
    capacityHistory?: readonly CapacityHistoryPoint[];
    trend?: CapacityTrend;
  };

  let {
    id = 'battery-health',
    currentFullCapacityWh = null,
    designCapacityWh = null,
    hardwareCycleCount = null,
    capacityHistory = [],
    trend = 'insufficient',
  }: Props = $props();

  const available = (value: number | null): value is number =>
    value !== null && Number.isFinite(value);
  const hasCapacity = (
    point: CapacityHistoryPoint,
  ): point is CapacityHistoryPoint & { fullCapacityWh: number } =>
    available(point.fullCapacityWh);
  const healthPercent = $derived(
    available(currentFullCapacityWh) &&
      available(designCapacityWh) &&
      designCapacityWh > 0
      ? (currentFullCapacityWh / designCapacityWh) * 100
      : null,
  );
  const plottedPoints = $derived(capacityHistory.filter(hasCapacity));
  const chartValues = $derived(plottedPoints.map((point) => point.fullCapacityWh));
  const chartMin = $derived(Math.min(...chartValues));
  const chartMax = $derived(Math.max(...chartValues));
  const chartRange = $derived(chartMax - chartMin || 1);
  const chartPath = $derived(
    plottedPoints
      .map((point, index) => {
        const x =
          plottedPoints.length === 1 ? 50 : (index / (plottedPoints.length - 1)) * 100;
        const y = 100 - ((point.fullCapacityWh - chartMin) / chartRange) * 100;
        return `${index === 0 ? 'M' : 'L'} ${x} ${y}`;
      })
      .join(' '),
  );

  const trendCopy: Record<CapacityTrend, { label: string; description: string }> = {
    stable: {
      label: 'Stable capacity',
      description: 'The available history does not show a meaningful capacity decline.',
    },
    degrading: {
      label: 'Capacity declining',
      description: 'The available history supports a conservative decline signal.',
    },
    noisy: {
      label: 'Trend is noisy',
      description:
        'Capacity readings vary too much for a reliable degradation conclusion.',
    },
    insufficient: {
      label: 'Insufficient history',
      description:
        'More recorded capacity observations are needed before assessing a trend.',
    },
  };
</script>

<section class="health-view" aria-labelledby={`${id}-title`}>
  <header>
    <p class="health-view__eyebrow">Battery health</p>
    <h2 id={`${id}-title`}>Capacity and wear</h2>
    <p class="health-view__description">
      Health is calculated only when both current maximum and design capacity are
      available.
    </p>
  </header>

  <dl class="health-view__metrics">
    <div>
      <dt>Current maximum capacity</dt>
      <dd>
        {available(currentFullCapacityWh)
          ? `${currentFullCapacityWh.toFixed(1)} Wh`
          : 'Unavailable'}
      </dd>
    </div>
    <div>
      <dt>Design capacity</dt>
      <dd>
        {available(designCapacityWh)
          ? `${designCapacityWh.toFixed(1)} Wh`
          : 'Unavailable'}
      </dd>
    </div>
    <div>
      <dt>Health</dt>
      <dd>{healthPercent === null ? 'Unavailable' : `${healthPercent.toFixed(1)}%`}</dd>
    </div>
    <div>
      <dt>Hardware cycle count</dt>
      <dd>
        {available(hardwareCycleCount)
          ? Math.round(hardwareCycleCount)
          : 'Not supported by this battery'}
      </dd>
    </div>
  </dl>

  <section class="health-view__history" aria-labelledby={`${id}-history-title`}>
    <div class="health-view__history-heading">
      <div>
        <h3 id={`${id}-history-title`}>Capacity history</h3>
        <p>Only recorded maximum-capacity readings are plotted.</p>
      </div>
      <span class={`health-view__trend health-view__trend--${trend}`}
        >{trendCopy[trend].label}</span
      >
    </div>
    <p class="health-view__trend-description">{trendCopy[trend].description}</p>

    {#if plottedPoints.length === 0}
      <div class="health-view__placeholder" role="status">
        No recorded capacity history is available.
      </div>
    {:else}
      <svg
        class="health-view__chart"
        viewBox="0 0 100 100"
        preserveAspectRatio="none"
        role="img"
        aria-label={`Capacity history with ${plottedPoints.length} recorded reading${plottedPoints.length === 1 ? '' : 's'}.`}
      >
        <path
          class="health-view__chart-line"
          d={chartPath}
          vector-effect="non-scaling-stroke"
        />
      </svg>
      <table>
        <caption>Recorded capacity history</caption>
        <thead
          ><tr><th scope="col">Recorded</th><th scope="col">Maximum capacity</th></tr
          ></thead
        >
        <tbody>
          {#each plottedPoints as point (`${point.timestamp}`)}
            <tr
              ><th scope="row">{new Date(point.timestamp).toLocaleDateString()}</th><td
                >{point.fullCapacityWh.toFixed(1)} Wh</td
              ></tr
            >
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
</section>

<style>
  .health-view {
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: 1.25rem;
    background: var(--color-surface);
  }
  .health-view__eyebrow,
  h2,
  h3,
  p,
  dl {
    margin: 0;
  }
  .health-view__eyebrow {
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
  .health-view__description,
  .health-view__history-heading p,
  .health-view__trend-description {
    margin-top: 0.4rem;
    color: var(--color-text-secondary);
    font-size: 0.88rem;
    line-height: 1.45;
  }
  .health-view__metrics {
    display: grid;
    grid-template-columns: repeat(4, minmax(9rem, 1fr));
    gap: 0.75rem;
    margin-top: 1.1rem;
  }
  .health-view__metrics div {
    min-height: 5.7rem;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.75rem;
    padding: 0.8rem;
    background: var(--color-surface-raised);
  }
  dt {
    color: var(--color-text-secondary);
    font-size: 0.75rem;
    font-weight: 650;
  }
  dd {
    margin: 0.45rem 0 0;
    font-size: 1rem;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .health-view__history {
    margin-top: 1rem;
    border-top: 1px solid var(--color-border-subtle);
    padding-top: 1rem;
  }
  .health-view__history-heading {
    display: flex;
    gap: 1rem;
    align-items: flex-start;
    justify-content: space-between;
  }
  h3 {
    font-size: 0.95rem;
  }
  .health-view__trend {
    flex: none;
    border-radius: 999px;
    padding: 0.32rem 0.55rem;
    font-size: 0.76rem;
    font-weight: 700;
  }
  .health-view__trend--stable {
    color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent), transparent 86%);
  }
  .health-view__trend--degrading {
    color: var(--color-warning);
    background: color-mix(in srgb, var(--color-warning), transparent 86%);
  }
  .health-view__trend--noisy,
  .health-view__trend--insufficient {
    color: var(--color-status);
    background: color-mix(in srgb, var(--color-status), transparent 86%);
  }
  .health-view__placeholder {
    display: grid;
    min-height: 7rem;
    margin-top: 0.8rem;
    place-items: center;
    border: 1px dashed var(--color-border-subtle);
    border-radius: 0.75rem;
    padding: 1rem;
    color: var(--color-text-secondary);
    text-align: center;
  }
  .health-view__chart {
    display: block;
    width: 100%;
    height: 8rem;
    margin-top: 0.8rem;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.65rem;
    padding: 0.7rem;
    background: color-mix(in srgb, var(--color-surface-raised), transparent 25%);
  }
  .health-view__chart-line {
    fill: none;
    stroke: var(--color-accent);
    stroke-width: 2;
  }
  table {
    width: 100%;
    margin-top: 0.7rem;
    border-collapse: collapse;
    font-size: 0.84rem;
  }
  caption {
    padding-bottom: 0.5rem;
    color: var(--color-text-secondary);
    text-align: left;
  }
  th,
  td {
    border-bottom: 1px solid var(--color-border-subtle);
    padding: 0.55rem;
    text-align: left;
  }
  thead th {
    color: var(--color-text-secondary);
    font-size: 0.72rem;
  }
  @media (max-width: 48rem) {
    .health-view__metrics {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .health-view__history-heading {
      display: grid;
    }
  }
</style>
