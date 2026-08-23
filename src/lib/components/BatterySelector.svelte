<script lang="ts">
  export type BatteryOption = { id: string; label: string; disabled?: boolean };

  type Props = {
    batteries: BatteryOption[];
    selectedId: string;
    label?: string;
    onSelect?: (id: string) => void;
  };

  let { batteries, selectedId, label = 'Battery', onSelect }: Props = $props();

  function selectBattery(event: Event) {
    onSelect?.((event.currentTarget as HTMLSelectElement).value);
  }
</script>

<label class="battery-selector">
  <span class="battery-selector__label">{label}</span>
  <select value={selectedId} onchange={selectBattery} aria-label={label}>
    {#each batteries as battery (battery.id)}
      <option value={battery.id} disabled={battery.disabled}>{battery.label}</option>
    {/each}
  </select>
</label>

<style>
  .battery-selector {
    display: grid;
    gap: 0.35rem;
    color: var(--color-text-secondary);
    font-size: 0.82rem;
    font-weight: 650;
  }

  select {
    min-width: 11rem;
    max-width: 100%;
    color-scheme: dark;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.7rem;
    padding: 0.55rem 2.2rem 0.55rem 0.7rem;
    color: var(--color-text-primary);
    background-color: var(--color-surface-raised);
  }
</style>
