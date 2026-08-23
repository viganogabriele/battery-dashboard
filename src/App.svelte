<script lang="ts">
  import { formatRelativeTime } from './lib/formatters';

  const lastUpdated = new Date();
</script>

<svelte:head>
  <title>Battery Dashboard</title>
</svelte:head>

<main class="app-shell">
  <header class="app-header">
    <div>
      <p class="eyebrow">Local-first Linux utility</p>
      <h1>Battery Dashboard</h1>
      <p class="lede">
        A private battery history for your laptop, built one reliable layer at a time.
      </p>
    </div>
    <span class="status" aria-label="Application status: setup in progress"
      >Setup in progress</span
    >
  </header>

  <section class="welcome-card" aria-labelledby="phase-one-title">
    <p class="eyebrow">Phase 1</p>
    <h2 id="phase-one-title">Foundation is ready</h2>
    <p>
      The interface is intentionally quiet for now. Live battery data, charts, and
      history will arrive in the next phases.
    </p>
    <dl class="details">
      <div>
        <dt>Data source</dt>
        <dd>Not connected yet</dd>
      </div>
      <div>
        <dt>Last updated</dt>
        <dd>{formatRelativeTime(lastUpdated)}</dd>
      </div>
      <div>
        <dt>Privacy</dt>
        <dd>Local only</dd>
      </div>
    </dl>
  </section>
</main>
