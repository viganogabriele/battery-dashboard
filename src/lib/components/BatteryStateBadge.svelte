<script lang="ts">
  import type { AggregateBatteryState } from '../domain/battery';

  type Props = { state: AggregateBatteryState };

  let { state }: Props = $props();

  const labels: Record<AggregateBatteryState, string> = {
    charging: 'Charging',
    discharging: 'On battery',
    full: 'Fully charged',
    idle: 'Plugged in',
    mixed: 'Mixed battery states',
    unknown: 'State unavailable',
  };
</script>

<span
  class={`battery-state battery-state--${state}`}
  aria-label={`Battery state: ${labels[state]}`}
>
  <span class="battery-state__dot" aria-hidden="true"></span>
  {labels[state]}
</span>

<style>
  .battery-state {
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    border: 1px solid var(--state-color);
    border-radius: 999px;
    padding: 0.35rem 0.65rem;
    color: var(--state-color);
    font-size: 0.82rem;
    font-weight: 650;
    white-space: nowrap;
  }

  .battery-state__dot {
    width: 0.48rem;
    height: 0.48rem;
    border-radius: 50%;
    background: currentColor;
  }

  .battery-state--charging,
  .battery-state--full {
    --state-color: var(--color-accent);
  }

  .battery-state--discharging {
    --state-color: var(--color-warning);
  }

  .battery-state--idle,
  .battery-state--mixed,
  .battery-state--unknown {
    --state-color: var(--color-status);
  }
</style>
