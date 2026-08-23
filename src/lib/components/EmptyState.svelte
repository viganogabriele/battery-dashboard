<script lang="ts">
  type Props = {
    title: string;
    message: string;
    hint?: string;
  };

  let { title, message, hint }: Props = $props();
</script>

<section class="empty-state" aria-labelledby="empty-state-title">
  <div class="empty-state__icon" aria-hidden="true">⌁</div>
  <div>
    <h2 id="empty-state-title">{title}</h2>
    <p>{message}</p>
    {#if hint}<p class="empty-state__hint">{hint}</p>{/if}
  </div>
</section>

<style>
  .empty-state {
    display: flex;
    gap: 1rem;
    align-items: flex-start;
    border: 1px dashed var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: clamp(1.25rem, 4vw, 2rem);
    color: var(--color-text-secondary);
    background: color-mix(in srgb, var(--color-surface), transparent 25%);
  }

  .empty-state__icon {
    display: grid;
    width: 2.3rem;
    height: 2.3rem;
    flex: none;
    place-items: center;
    border-radius: 0.7rem;
    color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent), transparent 88%);
    font-size: 1.4rem;
  }

  h2,
  p {
    margin: 0;
  }

  h2 {
    color: var(--color-text-primary);
    font-size: 1.05rem;
  }

  p {
    max-width: 60ch;
    margin-top: 0.4rem;
    line-height: 1.5;
  }

  .empty-state__hint {
    color: var(--color-status);
    font-size: 0.9rem;
  }
</style>
