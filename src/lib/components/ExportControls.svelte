<script lang="ts">
  export type ExportDataType = 'raw-samples' | 'sessions' | 'summaries';
  export type ExportFormat = 'csv' | 'json';
  export type ExportRequest = {
    dataType: ExportDataType;
    format: ExportFormat;
    /** Optional until a native save destination is chosen. */
    destination?: string;
  };

  type Props = {
    id?: string;
    selectedDataType?: ExportDataType;
    selectedFormat?: ExportFormat;
    onExport?: (request: ExportRequest) => void;
  };

  let {
    id = 'export-controls',
    selectedDataType = 'raw-samples',
    selectedFormat = 'csv',
    onExport = () => {},
  }: Props = $props();

  let destination = $state('');

  const dataTypes: readonly { value: ExportDataType; label: string }[] = [
    { value: 'raw-samples', label: 'Raw samples' },
    { value: 'sessions', label: 'Sessions' },
    { value: 'summaries', label: 'Calendar summaries' },
  ];
</script>

<section class="export-controls" aria-labelledby={`${id}-title`}>
  <div>
    <p class="export-controls__eyebrow">Local export</p>
    <h2 id={`${id}-title`}>Export recorded data</h2>
    <p class="export-controls__description">
      Choose what to export, its format, and an explicit destination path. Existing
      files are never overwritten.
    </p>
  </div>
  <div class="export-controls__form">
    <label
      >Data type
      <select bind:value={selectedDataType} aria-label="Export data type">
        {#each dataTypes as dataType (dataType.value)}<option value={dataType.value}
            >{dataType.label}</option
          >{/each}
      </select>
    </label>
    <label
      >Format
      <select bind:value={selectedFormat} aria-label="Export format">
        <option value="csv">CSV</option><option value="json">JSON</option>
      </select>
    </label>
    <label class="export-controls__destination"
      >Destination path
      <input
        type="text"
        bind:value={destination}
        placeholder="/home/user/battery-history.csv"
        autocomplete="off"
        spellcheck="false"
        aria-label="Export destination path"
      />
    </label>
    <button
      type="button"
      onclick={() =>
        onExport({
          dataType: selectedDataType,
          format: selectedFormat,
          ...(destination.trim() ? { destination: destination.trim() } : {}),
        })}>Export</button
    >
  </div>
</section>

<style>
  .export-controls {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 1rem;
    align-items: start;
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: 1.25rem;
    background: var(--color-surface);
  }
  .export-controls__eyebrow,
  h2,
  p {
    margin: 0;
  }
  .export-controls__eyebrow {
    color: var(--color-accent);
    font-size: 0.72rem;
    font-weight: 750;
    letter-spacing: 0.1em;
    text-transform: uppercase;
  }
  h2 {
    margin-top: 0.18rem;
    font-size: 1rem;
  }
  .export-controls__description {
    max-width: 60ch;
    margin-top: 0.45rem;
    color: var(--color-text-secondary);
    font-size: 0.88rem;
    line-height: 1.45;
  }
  .export-controls__form {
    display: flex;
    min-width: 0;
    flex-wrap: wrap;
    gap: 0.6rem;
    align-items: end;
  }
  label {
    display: grid;
    min-width: 0;
    flex: 1 1 8.5rem;
    gap: 0.3rem;
    color: var(--color-text-secondary);
    font-size: 0.75rem;
    font-weight: 700;
  }
  select,
  button {
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.55rem;
    padding: 0.48rem 0.6rem;
  }
  button {
    background: var(--color-surface-raised);
  }
  select {
    width: 100%;
    min-width: 0;
    color: var(--color-text-primary);
    background-color: var(--color-surface-raised);
  }
  .export-controls__destination {
    flex-basis: 15rem;
  }
  input {
    width: 100%;
    min-width: 0;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.55rem;
    padding: 0.48rem 0.6rem;
    color: var(--color-text-primary);
    background: var(--color-surface-raised);
  }
  button {
    border-color: color-mix(in srgb, var(--color-accent), transparent 32%);
    color: var(--color-accent-ink);
    background: var(--color-accent);
    font: inherit;
    font-size: 0.86rem;
    font-weight: 700;
    cursor: pointer;
  }
</style>
