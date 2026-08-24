<script lang="ts">
  export type CalendarAggregation = 'daily' | 'weekly' | 'monthly';
  export type CalendarHistoryState =
    'charging' | 'discharging' | 'full' | 'unknown' | 'all';
  export type CalendarHistoryBattery = { id: string; label: string };
  export type CalendarHistoryPeriod = {
    id: string;
    label: string;
    observedSamples?: number | null;
    minimumPercentage?: number | null;
    maximumPercentage?: number | null;
    observedEnergyWh?: number | null;
    recordedDurationSeconds?: number | null;
  };

  type Props = {
    id?: string;
    periods?: readonly CalendarHistoryPeriod[];
    batteries?: readonly CalendarHistoryBattery[];
    selectedAggregation?: CalendarAggregation;
    selectedBatteryId?: string;
    selectedState?: CalendarHistoryState;
    startDate?: string;
    endDate?: string;
    loading?: boolean;
    unsupportedReason?: string | null;
    onAggregationChange?: (aggregation: CalendarAggregation) => void;
    onBatteryChange?: (batteryId: string) => void;
    onStateChange?: (state: CalendarHistoryState) => void;
    onStartDateChange?: (date: string) => void;
    onEndDateChange?: (date: string) => void;
  };

  let {
    id = 'calendar-history',
    periods = [],
    batteries = [],
    selectedAggregation = 'daily',
    selectedBatteryId = 'all-batteries',
    selectedState = 'all',
    startDate = '',
    endDate = '',
    loading = false,
    unsupportedReason = null,
    onAggregationChange = () => {},
    onBatteryChange = () => {},
    onStateChange = () => {},
    onStartDateChange = () => {},
    onEndDateChange = () => {},
  }: Props = $props();

  const aggregations: readonly CalendarAggregation[] = ['daily', 'weekly', 'monthly'];
  const states: readonly CalendarHistoryState[] = [
    'all',
    'charging',
    'discharging',
    'full',
    'unknown',
  ];
  const label = (value: string) =>
    value === 'all' ? 'All states' : value[0].toUpperCase() + value.slice(1);
  const provided = (value: number | null | undefined): value is number =>
    value !== null && value !== undefined && Number.isFinite(value);
  const formatDuration = (seconds: number) => {
    const minutes = Math.round(seconds / 60);
    const hours = Math.floor(minutes / 60);
    return hours > 0 ? `${hours}h ${minutes % 60}m` : `${minutes}m`;
  };
</script>

<section class="calendar-history" aria-labelledby={`${id}-title`}>
  <header class="calendar-history__header">
    <div>
      <p class="calendar-history__eyebrow">Calendar history</p>
      <h2 id={`${id}-title`}>Recorded battery history</h2>
      <p class="calendar-history__description">
        Each value is derived only from stored local samples. Missing collection stays
        visible instead of being estimated.
      </p>
    </div>
    <div class="calendar-history__ranges" aria-label="Aggregation period">
      {#each aggregations as aggregation (aggregation)}<button
          type="button"
          aria-pressed={selectedAggregation === aggregation}
          onclick={() => onAggregationChange(aggregation)}>{aggregation}</button
        >{/each}
    </div>
  </header>
  <div class="calendar-history__filters" aria-label="History filters">
    <label
      >Battery<select
        value={selectedBatteryId}
        onchange={(event) => onBatteryChange(event.currentTarget.value)}
        ><option value="all-batteries">All batteries</option
        >{#each batteries as battery (battery.id)}<option value={battery.id}
            >{battery.label}</option
          >{/each}</select
      ></label
    ><label
      >State<select
        value={selectedState}
        onchange={(event) =>
          onStateChange(event.currentTarget.value as CalendarHistoryState)}
        >{#each states as state (state)}<option value={state}>{label(state)}</option
          >{/each}</select
      ></label
    ><label
      >From<input
        type="date"
        value={startDate}
        onchange={(event) => onStartDateChange(event.currentTarget.value)}
      /></label
    ><label
      >To<input
        type="date"
        value={endDate}
        onchange={(event) => onEndDateChange(event.currentTarget.value)}
      /></label
    >
  </div>
  {#if loading}<div class="calendar-history__placeholder" role="status">
      Loading local calendar history…
    </div>
  {:else if unsupportedReason}<div class="calendar-history__placeholder" role="status">
      Calendar history is unavailable. {unsupportedReason}
    </div>
  {:else if periods.length === 0}<div
      class="calendar-history__placeholder"
      role="status"
    >
      No recorded calendar history matches these filters.
    </div>
  {:else}<div class="calendar-history__table-wrap">
      <table>
        <caption>Calendar history by {selectedAggregation} period</caption><thead
          ><tr
            ><th scope="col">Period</th><th scope="col">Samples</th><th scope="col"
              >Minimum</th
            ><th scope="col">Maximum</th><th scope="col">Recorded time</th><th
              scope="col">Observed energy</th
            ></tr
          ></thead
        ><tbody
          >{#each periods as period (period.id)}<tr
              ><th scope="row">{period.label}</th><td
                >{provided(period.observedSamples) ? period.observedSamples : '—'}</td
              ><td
                >{provided(period.minimumPercentage)
                  ? `${period.minimumPercentage.toFixed(0)}%`
                  : '—'}</td
              ><td
                >{provided(period.maximumPercentage)
                  ? `${period.maximumPercentage.toFixed(0)}%`
                  : '—'}</td
              ><td
                >{provided(period.recordedDurationSeconds)
                  ? formatDuration(period.recordedDurationSeconds)
                  : '—'}</td
              ><td
                >{provided(period.observedEnergyWh)
                  ? `${period.observedEnergyWh.toFixed(1)} Wh`
                  : '—'}</td
              ></tr
            >{/each}</tbody
        >
      </table>
    </div>{/if}
</section>

<style>
  .calendar-history {
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: 1.25rem;
    background: var(--color-surface);
  }
  .calendar-history__header {
    display: flex;
    gap: 1rem;
    align-items: flex-start;
    justify-content: space-between;
  }
  .calendar-history__eyebrow,
  h2,
  p {
    margin: 0;
  }
  .calendar-history__eyebrow {
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
  .calendar-history__description {
    max-width: 60ch;
    margin-top: 0.4rem;
    color: var(--color-text-secondary);
    font-size: 0.88rem;
    line-height: 1.45;
  }
  .calendar-history__ranges {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    gap: 0.35rem;
  }
  .calendar-history__ranges button {
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
  .calendar-history__ranges button[aria-pressed='true'] {
    border-color: color-mix(in srgb, var(--color-accent), transparent 35%);
    color: var(--color-accent-ink);
    background: var(--color-accent);
  }
  .calendar-history__filters {
    display: grid;
    grid-template-columns: repeat(4, minmax(8rem, 1fr));
    gap: 0.7rem;
    margin-top: 1.1rem;
  }
  .calendar-history__filters label {
    display: grid;
    gap: 0.3rem;
    color: var(--color-text-secondary);
    font-size: 0.78rem;
    font-weight: 700;
  }
  .calendar-history__filters select,
  .calendar-history__filters input {
    width: 100%;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.55rem;
    padding: 0.45rem 0.55rem;
    color: var(--color-text-primary);
    background: var(--color-surface-raised);
  }
  .calendar-history__placeholder {
    display: grid;
    min-height: 9rem;
    margin-top: 1rem;
    place-items: center;
    border: 1px dashed var(--color-border-subtle);
    border-radius: 0.75rem;
    padding: 1rem;
    color: var(--color-text-secondary);
    text-align: center;
  }
  .calendar-history__table-wrap {
    margin-top: 1rem;
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.84rem;
  }
  caption {
    padding-bottom: 0.65rem;
    color: var(--color-text-secondary);
    text-align: left;
  }
  th,
  td {
    border-bottom: 1px solid var(--color-border-subtle);
    padding: 0.65rem;
    text-align: left;
    white-space: nowrap;
  }
  thead th {
    color: var(--color-text-secondary);
    font-size: 0.72rem;
  }
  tbody th {
    color: var(--color-text-primary);
  }
  @media (max-width: 42rem) {
    .calendar-history__header {
      display: grid;
    }
    .calendar-history__filters {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
</style>
