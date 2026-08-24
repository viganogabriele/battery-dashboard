<script lang="ts">
  export type Anomaly = {
    kind: 'unusual-power' | 'rapid-discharge' | 'interrupted-charge';
    recordedAt: string;
    startedAt: string | null;
    severity: 'medium' | 'high';
    confidence: 'medium' | 'high';
    observedValue: number | null;
    baselineValue: number | null;
    unit: string;
    explanation: string;
  };

  export type AnomalyReport = {
    availability: 'available' | 'insufficient' | 'unavailable';
    unavailableReason: string | null;
    rangeHours: number;
    observedSamples: number;
    powerSamples: number;
    dischargeIntervals: number;
    chargingTransitions: number;
    anomalies: readonly Anomaly[];
  };

  type Props = {
    report: AnomalyReport | null;
    loading?: boolean;
    rangeHours?: 24 | 168 | 720;
    onRangeChange?: (range: 24 | 168 | 720) => void;
    onRefresh?: () => void;
  };

  let {
    report,
    loading = false,
    rangeHours = 24,
    onRangeChange = () => undefined,
    onRefresh = () => undefined,
  }: Props = $props();

  const rangeOptions: readonly { value: 24 | 168 | 720; label: string }[] = [
    { value: 24, label: '24 hours' },
    { value: 168, label: '7 days' },
    { value: 720, label: '30 days' },
  ];

  const reasonCopy: Record<string, string> = {
    'recorder-disabled':
      'Enable recording in Settings before local patterns can be checked.',
    'too-few-samples':
      'More recorded samples are needed before checking for anomalies.',
    'too-few-baseline-values':
      'There are samples, but not enough comparable readings for a reliable baseline.',
    'no-usable-metrics':
      'The recorded samples do not expose the power or charge metrics needed for this check.',
    'multiple-batteries': 'Select one physical battery to inspect its behaviour.',
  };

  function title(kind: Anomaly['kind']): string {
    return {
      'unusual-power': 'Unusual power draw',
      'rapid-discharge': 'Rapid discharge',
      'interrupted-charge': 'Interrupted charge',
    }[kind];
  }

  function time(value: string): string {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(value));
  }
</script>

<section class="insights-view" aria-labelledby="insights-title">
  <header class="insights-view__header">
    <div>
      <p class="insights-view__eyebrow">Observed behaviour</p>
      <h2 id="insights-title">Battery insights</h2>
      <p>
        Findings are calculated locally from contiguous recorded samples. Gaps,
        suspended time, and missing values are never guessed.
      </p>
    </div>
    <button type="button" onclick={onRefresh} disabled={loading}>
      {loading ? 'Checking…' : 'Refresh'}
    </button>
  </header>

  <div class="insights-view__ranges" aria-label="Insight time range">
    {#each rangeOptions as option (option.value)}
      <button
        type="button"
        class:insights-view__range--active={rangeHours === option.value}
        aria-pressed={rangeHours === option.value}
        onclick={() => onRangeChange(option.value)}>{option.label}</button
      >
    {/each}
  </div>

  {#if loading}
    <p class="insights-view__empty" role="status">Reading local recorded history…</p>
  {:else if !report}
    <p class="insights-view__empty" role="status">
      Insights are available in the desktop application after a local history check.
    </p>
  {:else if report.availability !== 'available'}
    <p class="insights-view__empty" role="status">
      {reasonCopy[report.unavailableReason ?? ''] ??
        'No reliable insight can be produced from the available local history.'}
    </p>
  {:else if report.anomalies.length === 0}
    <p class="insights-view__empty insights-view__empty--clear" role="status">
      No unusual behaviour was found in this observed period.
    </p>
  {:else}
    <ol class="insights-view__findings">
      {#each report.anomalies as anomaly (`${anomaly.kind}-${anomaly.recordedAt}`)}
        <li>
          <div>
            <strong>{title(anomaly.kind)}</strong>
            <p>{anomaly.explanation}</p>
            <time datetime={anomaly.recordedAt}>{time(anomaly.recordedAt)}</time>
          </div>
          <span
            class={`insights-view__severity insights-view__severity--${anomaly.severity}`}
          >
            {anomaly.severity} · {anomaly.confidence} confidence
          </span>
        </li>
      {/each}
    </ol>
  {/if}

  {#if report}
    <footer>
      {report.observedSamples} recorded samples · {report.powerSamples} power readings ·
      {report.dischargeIntervals} contiguous discharge intervals
    </footer>
  {/if}
</section>

<style>
  .insights-view {
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: 1.1rem;
    background: var(--color-surface);
  }
  .insights-view__header {
    display: flex;
    gap: 1rem;
    align-items: flex-start;
    justify-content: space-between;
  }
  .insights-view__header p,
  .insights-view__findings p,
  .insights-view__eyebrow {
    margin: 0;
  }
  .insights-view__eyebrow {
    color: var(--color-accent);
    font-size: 0.72rem;
    font-weight: 750;
    letter-spacing: 0.1em;
    text-transform: uppercase;
  }
  h2 {
    margin: 0.18rem 0 0;
    font-size: 1.05rem;
  }
  .insights-view__header div > p:last-child {
    max-width: 62ch;
    margin-top: 0.42rem;
    color: var(--color-text-secondary);
    font-size: 0.88rem;
    line-height: 1.45;
  }
  button {
    flex: none;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.55rem;
    padding: 0.45rem 0.62rem;
    color: var(--color-text-primary);
    background: var(--color-surface-raised);
    font: inherit;
    font-size: 0.78rem;
    font-weight: 700;
    cursor: pointer;
  }
  button:disabled {
    cursor: wait;
    opacity: 0.6;
  }
  .insights-view__ranges {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-top: 1rem;
  }
  .insights-view__range--active {
    border-color: color-mix(in srgb, var(--color-accent), transparent 32%);
    color: var(--color-accent-ink);
    background: var(--color-accent);
  }
  .insights-view__empty {
    margin: 1rem 0 0;
    border: 1px dashed var(--color-border-subtle);
    border-radius: 0.7rem;
    padding: 1rem;
    color: var(--color-text-secondary);
    line-height: 1.45;
  }
  .insights-view__empty--clear {
    border-color: color-mix(in srgb, var(--color-accent), transparent 55%);
  }
  .insights-view__findings {
    display: grid;
    gap: 0.55rem;
    margin: 1rem 0 0;
    padding: 0;
    list-style: none;
  }
  .insights-view__findings li {
    display: flex;
    min-width: 0;
    gap: 0.85rem;
    align-items: flex-start;
    justify-content: space-between;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.7rem;
    padding: 0.82rem;
    background: var(--color-surface-raised);
  }
  .insights-view__findings strong {
    font-size: 0.9rem;
  }
  .insights-view__findings p,
  time {
    display: block;
    margin-top: 0.28rem;
    color: var(--color-text-secondary);
    font-size: 0.8rem;
    line-height: 1.4;
  }
  .insights-view__severity {
    flex: none;
    border-radius: 999px;
    padding: 0.27rem 0.45rem;
    font-size: 0.68rem;
    font-weight: 750;
    white-space: nowrap;
  }
  .insights-view__severity--medium {
    color: var(--color-warning);
    background: color-mix(in srgb, var(--color-warning), transparent 86%);
  }
  .insights-view__severity--high {
    color: var(--color-danger);
    background: color-mix(in srgb, var(--color-danger), transparent 86%);
  }
  footer {
    margin-top: 0.8rem;
    color: var(--color-text-secondary);
    font-size: 0.75rem;
  }
  @media (max-width: 38rem) {
    .insights-view__header,
    .insights-view__findings li {
      display: grid;
    }
  }
</style>
