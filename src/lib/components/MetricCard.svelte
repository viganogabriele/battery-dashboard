<script lang="ts">
  import type { MetricSource } from '../domain/battery';

  type Props = {
    label: string;
    value?: string | number | null;
    unit?: string;
    source?: MetricSource;
    stale?: boolean;
    unavailableLabel?: string;
  };

  let {
    label,
    value = null,
    unit,
    source = 'unavailable',
    stale = false,
    unavailableLabel = 'Unavailable',
  }: Props = $props();

  const sourceLabels: Record<MetricSource, string> = {
    upower: 'UPower',
    sysfs: 'Linux sysfs',
    derived: 'Calculated locally',
    simulated: 'Simulated data',
    unavailable: 'Not available',
  };
</script>

<article class:metric-card--stale={stale} class="metric-card" aria-label={label}>
  <p class="metric-card__label">{label}</p>
  {#if value !== null && value !== undefined}
    <p class="metric-card__value">
      {value}<span class="metric-card__unit">{unit ?? ''}</span>
    </p>
  {:else}
    <p class="metric-card__value metric-card__value--unavailable">{unavailableLabel}</p>
  {/if}
  <footer class="metric-card__meta">
    <span>{sourceLabels[source]}</span>
    {#if stale}<span class="metric-card__stale">May be outdated</span>{/if}
  </footer>
</article>

<style>
  .metric-card {
    display: grid;
    min-height: 8.5rem;
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: 1rem;
    background: var(--color-surface);
  }

  .metric-card--stale {
    border-style: dashed;
  }

  .metric-card__label,
  .metric-card__meta {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: 0.8rem;
  }

  .metric-card__label {
    font-weight: 650;
  }

  .metric-card__value {
    align-self: center;
    margin: 0.25rem 0;
    color: var(--color-text-primary);
    font-size: 1.8rem;
    font-weight: 700;
    letter-spacing: -0.04em;
  }

  .metric-card__value--unavailable {
    color: var(--color-text-secondary);
    font-size: 1rem;
    font-weight: 550;
    letter-spacing: normal;
  }

  .metric-card__unit {
    margin-left: 0.25rem;
    color: var(--color-text-secondary);
    font-size: 0.6em;
    font-weight: 600;
    letter-spacing: normal;
  }

  .metric-card__meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }

  .metric-card__stale {
    color: var(--color-status);
  }
</style>
