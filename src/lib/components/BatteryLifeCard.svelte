<script lang="ts">
  export type BatteryLifeEvidence = 'sufficient' | 'insufficient';
  export type BatteryLifeConfidence = 'none' | 'low' | 'moderate' | 'high';

  export type BatteryLifeHeadline = {
    evidence: BatteryLifeEvidence;
    confidence: BatteryLifeConfidence;
    sessionCount: number;
    averageMinutes: number | null;
    medianMinutes: number | null;
    minMinutes: number | null;
    maxMinutes: number | null;
  };

  export type DurationStats = {
    count: number;
    averageMinutes: number;
    medianMinutes: number;
    minMinutes: number;
    maxMinutes: number;
  };

  export type StartingChargeBand = {
    bandStartPercent: number;
    bandEndPercent: number;
    isFullChargeBand: boolean;
    allSessions: DurationStats | null;
    fullyDrained: DurationStats | null;
  };

  export type BatteryLifeEstimate = {
    fullChargeMinPercent: number;
    fullyDrainedMaxPercent: number;
    headline: BatteryLifeHeadline;
    bands: StartingChargeBand[];
    totalSessionCount: number;
    earliestSessionStartedAt: string | null;
    latestSessionEndedAt: string | null;
  };

  type Props = {
    id?: string;
    estimate?: BatteryLifeEstimate | null;
    loading?: boolean;
    unsupportedReason?: string | null;
  };

  let {
    id = 'battery-life',
    estimate = null,
    loading = false,
    unsupportedReason = null,
  }: Props = $props();

  function formatMinutes(minutes: number): string {
    const wholeMinutes = Math.round(minutes);
    const hours = Math.floor(wholeMinutes / 60);
    const remainder = wholeMinutes % 60;
    return hours > 0 ? `${hours}h ${remainder}m` : `${remainder}m`;
  }

  function formatDate(value: string | null): string | null {
    if (!value) return null;
    const date = new Date(value);
    return Number.isFinite(date.getTime())
      ? new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' }).format(date)
      : null;
  }

  function confidenceLabel(confidence: BatteryLifeConfidence): string {
    switch (confidence) {
      case 'high':
        return 'High confidence';
      case 'moderate':
        return 'Moderate confidence';
      case 'low':
        return 'Early estimate';
      default:
        return 'Not enough data yet';
    }
  }

  function sessionWord(count: number): string {
    return count === 1 ? 'discharge' : 'discharges';
  }

  let headline = $derived(estimate?.headline ?? null);
  let fullChargeMinPercent = $derived(estimate?.fullChargeMinPercent ?? 95);
  let fullyDrainedMaxPercent = $derived(estimate?.fullyDrainedMaxPercent ?? 20);
  let dateRange = $derived(
    estimate?.earliestSessionStartedAt && estimate?.latestSessionEndedAt
      ? `${formatDate(estimate.earliestSessionStartedAt)} – ${formatDate(estimate.latestSessionEndedAt)}`
      : null,
  );
  let bandsWithEvidence = $derived(
    (estimate?.bands ?? []).filter((band) => band.allSessions !== null),
  );
</script>

<section class="battery-life" aria-labelledby={`${id}-title`}>
  {#if loading}
    <p class="battery-life__eyebrow">Estimated battery life</p>
    <p class="battery-life__compact" role="status">Loading recorded discharges…</p>
  {:else if unsupportedReason}
    <p class="battery-life__eyebrow">Estimated battery life</p>
    <p class="battery-life__compact" role="status">
      Not available. {unsupportedReason}
    </p>
  {:else if !headline || headline.evidence === 'insufficient'}
    <p class="battery-life__eyebrow">Estimated battery life on a full charge</p>
    <p class="battery-life__compact" role="status">
      Not enough data yet — record a few full discharges (starting at
      {fullChargeMinPercent}% or more and running down to
      {fullyDrainedMaxPercent}% or less) to see this.
    </p>
  {:else}
    <header>
      <p class="battery-life__eyebrow" id={`${id}-title`}>
        Estimated battery life on a full charge
      </p>
    </header>
    <p class="battery-life__headline">{formatMinutes(headline.averageMinutes ?? 0)}</p>
    <p class="battery-life__evidence">
      <span
        class="battery-life__confidence"
        class:battery-life__confidence--high={headline.confidence === 'high'}
        class:battery-life__confidence--moderate={headline.confidence === 'moderate'}
        class:battery-life__confidence--low={headline.confidence === 'low'}
        >{confidenceLabel(headline.confidence)}</span
      >
      <span class="battery-life__muted">
        · average of {headline.sessionCount} recorded full-charge {sessionWord(
          headline.sessionCount,
        )}{dateRange ? ` (${dateRange})` : ''}
      </span>
    </p>
    <p class="battery-life__range">
      Observed range: {formatMinutes(headline.minMinutes ?? 0)} to {formatMinutes(
        headline.maxMinutes ?? 0,
      )}
      · median {formatMinutes(headline.medianMinutes ?? 0)}
    </p>
    <p class="battery-life__method">
      Based only on completed discharge sessions that started at
      {fullChargeMinPercent}% or more and ran down to
      {fullyDrainedMaxPercent}% or less; nothing is extrapolated beyond what was
      actually recorded.
    </p>

    {#if bandsWithEvidence.length > 0}
      <details class="battery-life__bands">
        <summary>See duration by starting-charge band</summary>
        <div class="battery-life__table-wrap">
          <table>
            <thead>
              <tr>
                <th scope="col">Started at</th>
                <th scope="col">Sessions</th>
                <th scope="col">Average</th>
                <th scope="col">Median</th>
                <th scope="col">Range</th>
              </tr>
            </thead>
            <tbody>
              {#each bandsWithEvidence as band (band.bandStartPercent)}
                <tr class:battery-life__row--headline={band.isFullChargeBand}>
                  <th scope="row">{band.bandStartPercent}–{band.bandEndPercent}%</th>
                  <td>{band.allSessions?.count ?? 0}</td>
                  <td>{formatMinutes(band.allSessions?.averageMinutes ?? 0)}</td>
                  <td>{formatMinutes(band.allSessions?.medianMinutes ?? 0)}</td>
                  <td>
                    {formatMinutes(band.allSessions?.minMinutes ?? 0)} – {formatMinutes(
                      band.allSessions?.maxMinutes ?? 0,
                    )}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        <p class="battery-life__bands-note">
          Each row counts every completed discharge session that started in that range,
          regardless of how far it ran down; the headline number above uses only the top
          band's sessions that also reached
          {fullyDrainedMaxPercent}% or less.
        </p>
      </details>
    {/if}
  {/if}
</section>

<style>
  .battery-life {
    margin-top: 1rem;
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: 1.25rem;
    background: linear-gradient(
      160deg,
      color-mix(in srgb, var(--color-accent), transparent 92%),
      var(--color-surface)
    );
  }
  .battery-life__eyebrow,
  p {
    margin: 0;
  }
  .battery-life__eyebrow {
    color: var(--color-accent);
    font-size: 0.72rem;
    font-weight: 750;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .battery-life__compact {
    margin-top: 0.4rem;
    color: var(--color-text-secondary);
    font-size: 0.85rem;
    line-height: 1.4;
  }
  .battery-life__headline {
    margin-top: 0.3rem;
    color: var(--color-text-primary);
    font-size: 2.1rem;
    font-weight: 800;
    line-height: 1.05;
  }
  .battery-life__evidence {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.3rem;
    margin-top: 0.35rem;
    font-size: 0.8rem;
  }
  .battery-life__confidence {
    border-radius: 999px;
    padding: 0.14rem 0.5rem;
    color: var(--color-text-secondary);
    background: color-mix(in srgb, var(--color-text-secondary), transparent 85%);
    font-size: 0.7rem;
    font-weight: 750;
  }
  .battery-life__confidence--high {
    color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent), transparent 85%);
  }
  .battery-life__confidence--moderate {
    color: var(--color-status);
    background: color-mix(in srgb, var(--color-status), transparent 85%);
  }
  .battery-life__confidence--low {
    color: var(--color-warning);
    background: color-mix(in srgb, var(--color-warning), transparent 85%);
  }
  .battery-life__muted {
    color: var(--color-text-secondary);
  }
  .battery-life__range,
  .battery-life__method {
    margin-top: 0.4rem;
    color: var(--color-text-secondary);
    font-size: 0.78rem;
    line-height: 1.4;
  }
  .battery-life__method {
    max-width: 60ch;
  }
  .battery-life__bands {
    margin-top: 0.7rem;
  }
  .battery-life__bands summary {
    cursor: pointer;
    color: var(--color-accent);
    font-size: 0.78rem;
    font-weight: 700;
  }
  .battery-life__table-wrap {
    margin-top: 0.6rem;
    overflow-x: auto;
  }
  .battery-life__bands table {
    width: 100%;
    min-width: 26rem;
    border-collapse: collapse;
    font-size: 0.76rem;
  }
  .battery-life__bands th,
  .battery-life__bands td {
    border-bottom: 1px solid var(--color-border-subtle);
    padding: 0.32rem 0.4rem;
    text-align: left;
    white-space: nowrap;
  }
  .battery-life__bands thead th {
    color: var(--color-text-secondary);
    font-weight: 700;
  }
  .battery-life__row--headline th {
    color: var(--color-accent);
  }
  .battery-life__bands-note {
    margin-top: 0.5rem;
    color: var(--color-text-secondary);
    font-size: 0.72rem;
    line-height: 1.4;
  }

  @media (max-width: 30rem) {
    .battery-life__headline {
      font-size: 1.6rem;
    }
  }
</style>
