<script lang="ts">
  import { onMount } from 'svelte';

  import type { RecorderClient, RecorderStatus } from '../services/recorder-client';

  type Props = {
    client: RecorderClient;
    initialStatus?: RecorderStatus;
  };

  let { client, initialStatus }: Props = $props();
  let status = $state<RecorderStatus>({
    state: 'unsupported',
    lastRecordedAt: null,
    error: null,
  });
  let isRefreshing = $state(false);
  let isChanging = $state(false);

  onMount(() => {
    if (initialStatus) {
      status = initialStatus;
      return;
    }

    void refreshStatus();
  });

  const copy = $derived(statusCopy(status));
  const canToggle = $derived(status.state === 'enabled' || status.state === 'disabled');

  async function refreshStatus() {
    isRefreshing = true;

    try {
      status = await client.getStatus();
    } catch {
      status = {
        state: 'error',
        lastRecordedAt: null,
        error: 'The recorder status could not be read.',
      };
    } finally {
      isRefreshing = false;
    }
  }

  async function toggleRecorder() {
    if (!canToggle || isChanging) return;

    const enabled = status.state !== 'enabled';
    isChanging = true;
    status = {
      ...status,
      state: enabled ? 'enabling' : 'disabling',
      error: null,
    };

    try {
      status = await client.setEnabled(enabled);
    } catch {
      status = {
        ...status,
        state: 'error',
        error: 'The recorder setting could not be changed.',
      };
    } finally {
      isChanging = false;
    }
  }

  function statusCopy(currentStatus: RecorderStatus) {
    switch (currentStatus.state) {
      case 'unsupported':
        return {
          label: 'Not supported on this system',
          description: 'The background recorder is unavailable in this environment.',
        };
      case 'disabled':
        return {
          label: 'Recording is disabled',
          description: 'No background samples are collected while it is disabled.',
        };
      case 'enabling':
        return {
          label: 'Enabling recording',
          description: 'Applying the local setting…',
        };
      case 'disabling':
        return {
          label: 'Disabling recording',
          description: 'Applying the local setting…',
        };
      case 'enabled':
        return {
          label: 'Recording is active',
          description: 'The local background recorder is enabled.',
        };
      case 'error':
        return {
          label: 'Recorder needs attention',
          description: currentStatus.error ?? 'The recorder reported an unknown error.',
        };
    }
  }
</script>

<section class="recorder-settings" aria-labelledby="recorder-settings-title">
  <div class="recorder-settings__heading">
    <div>
      <p class="recorder-settings__eyebrow">History collection</p>
      <h2 id="recorder-settings-title">Background recorder</h2>
    </div>
    <span
      class:recorder-settings__status--error={status.state === 'error'}
      class="recorder-settings__status"
    >
      {copy.label}
    </span>
  </div>

  <p class="recorder-settings__description">{copy.description}</p>
  <p class="recorder-settings__privacy">
    Recording is opt-in and stays local to this device. It can be disabled at any time.
  </p>

  {#if canToggle}
    <button
      class="recorder-settings__button"
      type="button"
      disabled={isChanging}
      onclick={toggleRecorder}
    >
      {status.state === 'enabled' ? 'Disable recording' : 'Enable recording'}
    </button>
  {:else if status.state === 'error'}
    <button
      class="recorder-settings__button"
      type="button"
      disabled={isRefreshing}
      onclick={refreshStatus}
    >
      {isRefreshing ? 'Refreshing…' : 'Refresh status'}
    </button>
  {/if}
</section>

<style>
  .recorder-settings {
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: 1.25rem;
    background: var(--color-surface);
  }

  .recorder-settings__heading {
    display: flex;
    gap: 0.8rem;
    align-items: flex-start;
    justify-content: space-between;
  }

  .recorder-settings__eyebrow,
  h2,
  p {
    margin: 0;
  }

  .recorder-settings__eyebrow {
    color: var(--color-text-secondary);
    font-size: 0.72rem;
    font-weight: 750;
    letter-spacing: 0.1em;
    text-transform: uppercase;
  }

  h2 {
    margin-top: 0.18rem;
    color: var(--color-text-primary);
    font-size: 1rem;
  }

  .recorder-settings__status {
    flex: none;
    border-radius: 999px;
    padding: 0.32rem 0.55rem;
    color: var(--color-status);
    background: color-mix(in srgb, var(--color-status), transparent 85%);
    font-size: 0.76rem;
    font-weight: 700;
  }

  .recorder-settings__status--error {
    color: var(--color-danger, #c64d4d);
    background: color-mix(in srgb, var(--color-danger, #c64d4d), transparent 85%);
  }

  .recorder-settings__description,
  .recorder-settings__privacy {
    max-width: 62ch;
    margin-top: 0.72rem;
    color: var(--color-text-secondary);
    font-size: 0.9rem;
    line-height: 1.45;
  }

  .recorder-settings__privacy {
    color: var(--color-status);
  }

  .recorder-settings__button {
    margin-top: 1rem;
    border: 1px solid color-mix(in srgb, var(--color-accent), transparent 32%);
    border-radius: 0.6rem;
    padding: 0.55rem 0.75rem;
    color: var(--color-accent-ink);
    background: var(--color-accent);
    font: inherit;
    font-size: 0.88rem;
    font-weight: 700;
    cursor: pointer;
  }

  .recorder-settings__button:disabled {
    cursor: wait;
    opacity: 0.65;
  }
</style>
