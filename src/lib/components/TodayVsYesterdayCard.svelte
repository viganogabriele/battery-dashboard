<script lang="ts">
  export type DayUsageEvidence = 'sufficient' | 'insufficient';
  export type DayUsageInsufficientReason = 'no-recording' | 'too-few-samples';

  /** Every field is `null` unless recorded samples directly support it. */
  export type DayUsageSummary = {
    date: string;
    evidence: DayUsageEvidence;
    insufficientReason?: DayUsageInsufficientReason | null;
    sampleCount: number;
    elapsedSeconds: number;
    observedDurationSeconds: number | null;
    coverageRatio: number | null;
    startPercentage: number | null;
    endPercentage: number | null;
    percentageChange: number | null;
    energyChangeWh: number | null;
    averageDischargePowerWatts: number | null;
    averageChargePowerWatts: number | null;
    /** Set only for the aggregate "all batteries" view. */
    contributingBatteries?: number | null;
  };

  type Props = {
    id?: string;
    today?: DayUsageSummary | null;
    yesterday?: DayUsageSummary | null;
    loading?: boolean;
    unsupportedReason?: string | null;
  };

  let {
    id = 'day-usage',
    today = null,
    yesterday = null,
    loading = false,
    unsupportedReason = null,
  }: Props = $props();

  function formatDate(value: string): string {
    const date = new Date(`${value}T00:00:00`);
    return Number.isFinite(date.getTime())
      ? new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' }).format(date)
      : value;
  }

  function formatDuration(seconds: number): string {
    const wholeMinutes = Math.round(seconds / 60);
    const hours = Math.floor(wholeMinutes / 60);
    const remainder = wholeMinutes % 60;
    return hours > 0 ? `${hours}h ${remainder}m` : `${remainder}m`;
  }

  function formatPercent(value: number): string {
    return `${value > 0 ? '+' : ''}${value.toFixed(0)}%`;
  }

  function formatEnergy(value: number): string {
    return `${value > 0 ? '+' : ''}${value.toFixed(1)} Wh`;
  }

  function formatPower(value: number): string {
    return `${value.toFixed(1)} W`;
  }

  function formatCoverage(ratio: number): string {
    return `${Math.round(ratio * 100)}% of the day recorded`;
  }

  function insufficientMessage(day: DayUsageSummary): string {
    if (day.insufficientReason === 'no-recording') {
      return day.sampleCount === 0 && day.elapsedSeconds === 0
        ? 'This day has not started yet.'
        : 'No samples were recorded for this day.';
    }
    return `Only ${day.sampleCount} sample${day.sampleCount === 1 ? '' : 's'} recorded so far. At least 10 samples over 10 observed minutes are required.`;
  }
</script>

<section class="day-usage" aria-labelledby={`${id}-title`}>
  <header>
    <p class="day-usage__eyebrow">Observed answers</p>
    <h2 id={`${id}-title`}>Today vs yesterday</h2>
    <p class="day-usage__description">
      Built only from recorded samples for each local calendar day. Figures are omitted,
      never estimated, when a day has too little recorded coverage.
    </p>
  </header>

  {#if loading}
    <div class="day-usage__placeholder" role="status">Loading recorded usage…</div>
  {:else if unsupportedReason}
    <div class="day-usage__placeholder" role="status">
      Today/yesterday comparison is unavailable. {unsupportedReason}
    </div>
  {:else if !today || !yesterday}
    <div class="day-usage__placeholder" role="status">
      No recorded usage is available yet.
    </div>
  {:else}
    <div class="day-usage__days">
      {#each [{ label: 'Today', day: today }, { label: 'Yesterday', day: yesterday }] as entry (entry.label)}
        <article class="day-usage__day">
          <header>
            <span class="day-usage__day-label">{entry.label}</span>
            <span class="day-usage__day-date">{formatDate(entry.day.date)}</span>
          </header>
          {#if entry.day.evidence === 'insufficient'}
            <p class="day-usage__insufficient">{insufficientMessage(entry.day)}</p>
          {:else}
            <dl>
              <div>
                <dt>Observed coverage</dt>
                <dd>
                  {formatDuration(entry.day.observedDurationSeconds ?? 0)}
                  {#if entry.day.coverageRatio !== null}
                    <span class="day-usage__muted"
                      >· {formatCoverage(entry.day.coverageRatio)}</span
                    >
                  {/if}
                </dd>
              </div>
              {#if entry.day.percentageChange !== null}
                <div>
                  <dt>Charge change</dt>
                  <dd>
                    {formatPercent(entry.day.percentageChange)}
                    {#if entry.day.startPercentage !== null && entry.day.endPercentage !== null}
                      <span class="day-usage__muted"
                        >· {entry.day.startPercentage.toFixed(0)}% → {entry.day.endPercentage.toFixed(
                          0,
                        )}%</span
                      >
                    {/if}
                  </dd>
                </div>
              {/if}
              {#if entry.day.energyChangeWh !== null}
                <div>
                  <dt>Energy change</dt>
                  <dd>{formatEnergy(entry.day.energyChangeWh)}</dd>
                </div>
              {/if}
              {#if entry.day.averageDischargePowerWatts !== null}
                <div>
                  <dt>Average draw (discharging)</dt>
                  <dd class="day-usage__discharge">
                    {formatPower(entry.day.averageDischargePowerWatts)}
                  </dd>
                </div>
              {/if}
              {#if entry.day.averageChargePowerWatts !== null}
                <div>
                  <dt>Average draw (charging)</dt>
                  <dd class="day-usage__charge">
                    {formatPower(entry.day.averageChargePowerWatts)}
                  </dd>
                </div>
              {/if}
              {#if entry.day.contributingBatteries !== null && entry.day.contributingBatteries !== undefined}
                <div>
                  <dt>Batteries with enough evidence</dt>
                  <dd>{entry.day.contributingBatteries}</dd>
                </div>
              {/if}
            </dl>
          {/if}
        </article>
      {/each}
    </div>
  {/if}
</section>

<style>
  .day-usage {
    margin-top: 1rem;
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: 1.25rem;
    background: var(--color-surface);
  }
  .day-usage__eyebrow,
  h2,
  p,
  dt,
  dd {
    margin: 0;
  }
  .day-usage__eyebrow {
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
  .day-usage__description {
    max-width: 60ch;
    margin-top: 0.4rem;
    color: var(--color-text-secondary);
    font-size: 0.88rem;
    line-height: 1.45;
  }
  .day-usage__placeholder {
    display: grid;
    min-height: 6rem;
    margin-top: 0.75rem;
    place-items: center;
    border: 1px dashed var(--color-border-subtle);
    border-radius: 0.6rem;
    padding: 1rem;
    color: var(--color-text-secondary);
    text-align: center;
  }
  .day-usage__days {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.6rem;
    margin-top: 0.75rem;
  }
  .day-usage__day {
    min-width: 0;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.6rem;
    padding: 0.7rem;
    background: var(--color-surface-raised);
  }
  .day-usage__day > header {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.3rem;
  }
  .day-usage__day-label {
    color: var(--color-text-primary);
    font-size: 0.82rem;
    font-weight: 750;
  }
  .day-usage__day-date {
    color: var(--color-text-secondary);
    font-size: 0.72rem;
  }
  .day-usage__insufficient {
    margin-top: 0.5rem;
    color: var(--color-text-secondary);
    font-size: 0.78rem;
    line-height: 1.4;
  }
  dl {
    display: grid;
    gap: 0.45rem;
    margin-top: 0.55rem;
  }
  dt {
    color: var(--color-text-secondary);
    font-size: 0.7rem;
  }
  dd {
    margin-top: 0.12rem;
    color: var(--color-text-primary);
    font-size: 0.85rem;
  }
  .day-usage__muted {
    color: var(--color-text-secondary);
    font-size: 0.72rem;
  }
  .day-usage__discharge {
    color: var(--color-warning);
  }
  .day-usage__charge {
    color: var(--color-accent);
  }

  @media (max-width: 720px) {
    .day-usage__days {
      grid-template-columns: 1fr;
    }
  }
</style>
