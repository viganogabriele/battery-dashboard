<script lang="ts">
  export type PowerProfile = 'power-saver' | 'balanced' | 'performance';

  export type PowerProfileState = {
    availability: 'available' | 'unsupported' | 'unavailable';
    supported: boolean;
    activeProfile: PowerProfile | null;
    availableProfiles: readonly PowerProfile[];
    unavailableReason: string | null;
    error: string | null;
  };

  type Props = {
    state: PowerProfileState | null;
    loading?: boolean;
    changing?: boolean;
    onRefresh?: () => void;
    onSelect?: (profile: PowerProfile) => void;
  };

  let {
    state,
    loading = false,
    changing = false,
    onRefresh = () => undefined,
    onSelect = () => undefined,
  }: Props = $props();

  const labels: Record<PowerProfile, string> = {
    'power-saver': 'Power saver',
    balanced: 'Balanced',
    performance: 'Performance',
  };
</script>

<section class="power-profiles" aria-labelledby="power-profiles-title">
  <header>
    <div>
      <p class="power-profiles__eyebrow">System profile</p>
      <h2 id="power-profiles-title">Power profile</h2>
      <p>Uses the local power-profiles service only when Linux provides it.</p>
    </div>
    <button type="button" onclick={onRefresh} disabled={loading || changing}>
      {loading ? 'Checking…' : 'Refresh'}
    </button>
  </header>

  {#if !state}
    <p class="power-profiles__empty" role="status">
      Profile information has not been read yet.
    </p>
  {:else if state.availability !== 'available'}
    <p class="power-profiles__empty" role="status">
      {state.error ??
        'This Linux installation does not provide a usable power profile service.'}
    </p>
  {:else}
    <p class="power-profiles__active" role="status">
      Active profile: <strong
        >{state.activeProfile ? labels[state.activeProfile] : 'Unavailable'}</strong
      >
    </p>
    <div class="power-profiles__choices" aria-label="Choose a power profile">
      {#each state.availableProfiles as profile (profile)}
        <button
          type="button"
          class:power-profiles__choice--active={state.activeProfile === profile}
          aria-pressed={state.activeProfile === profile}
          disabled={changing || state.activeProfile === profile}
          onclick={() => onSelect(profile)}>{labels[profile]}</button
        >
      {/each}
    </div>
    <p class="power-profiles__note">
      Changes are confirmed by Linux after they are requested. No administrator
      privileges are used.
    </p>
  {/if}
</section>

<style>
  .power-profiles {
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: 1.1rem;
    background: var(--color-surface);
  }
  header {
    display: flex;
    gap: 1rem;
    align-items: flex-start;
    justify-content: space-between;
  }
  p {
    margin: 0;
  }
  .power-profiles__eyebrow {
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
  header div > p:last-child,
  .power-profiles__note {
    margin-top: 0.42rem;
    color: var(--color-text-secondary);
    font-size: 0.84rem;
    line-height: 1.45;
  }
  button {
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
    cursor: default;
    opacity: 0.62;
  }
  .power-profiles__empty,
  .power-profiles__active {
    margin-top: 1rem;
    border: 1px dashed var(--color-border-subtle);
    border-radius: 0.7rem;
    padding: 0.85rem;
    color: var(--color-text-secondary);
    font-size: 0.86rem;
    line-height: 1.45;
  }
  .power-profiles__active {
    border-style: solid;
    color: var(--color-text-primary);
  }
  .power-profiles__choices {
    display: flex;
    flex-wrap: wrap;
    gap: 0.45rem;
    margin-top: 0.7rem;
  }
  .power-profiles__choice--active {
    border-color: color-mix(in srgb, var(--color-accent), transparent 32%);
    color: var(--color-accent-ink);
    background: var(--color-accent);
  }
</style>
