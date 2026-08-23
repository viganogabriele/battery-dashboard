<script lang="ts">
  export type ExecutionContext = 'simulated-preview' | 'native-desktop';

  type ContextCopy = {
    eyebrow: string;
    title: string;
    description: string;
  };

  type Props = {
    executionContext: ExecutionContext;
  };

  const contextCopy: Record<ExecutionContext, ContextCopy> = {
    'simulated-preview': {
      eyebrow: 'Preview mode',
      title: 'Simulated battery data',
      description:
        'This screen uses sample readings. It does not read or store system battery data.',
    },
    'native-desktop': {
      eyebrow: 'Desktop mode',
      title: 'Native desktop window',
      description:
        'This screen is running in the desktop application. Each metric identifies whether its value is available.',
    },
  };

  let { executionContext }: Props = $props();
  let copy = $derived(contextCopy[executionContext]);
</script>

<aside class="execution-context" aria-labelledby="execution-context-title">
  <span class="execution-context__icon" aria-hidden="true">◌</span>
  <div>
    <p class="execution-context__eyebrow">{copy.eyebrow}</p>
    <h2 id="execution-context-title">{copy.title}</h2>
    <p class="execution-context__description">{copy.description}</p>
  </div>
</aside>

<style>
  .execution-context {
    display: flex;
    gap: 0.75rem;
    align-items: flex-start;
    border: 1px solid color-mix(in srgb, var(--color-status), transparent 52%);
    border-radius: 0.9rem;
    padding: 0.85rem 1rem;
    color: var(--color-text-secondary);
    background: color-mix(in srgb, var(--color-status), transparent 92%);
  }

  .execution-context__icon {
    display: grid;
    width: 1.5rem;
    height: 1.5rem;
    flex: none;
    place-items: center;
    border-radius: 50%;
    color: var(--color-status);
    background: color-mix(in srgb, var(--color-status), transparent 82%);
    font-size: 0.95rem;
    font-weight: 700;
  }

  .execution-context__eyebrow,
  h2,
  .execution-context__description {
    margin: 0;
  }

  .execution-context__eyebrow {
    color: var(--color-status);
    font-size: 0.72rem;
    font-weight: 750;
    letter-spacing: 0.1em;
    text-transform: uppercase;
  }

  h2 {
    margin-top: 0.18rem;
    color: var(--color-text-primary);
    font-size: 0.95rem;
  }

  .execution-context__description {
    max-width: 62ch;
    margin-top: 0.3rem;
    font-size: 0.88rem;
    line-height: 1.45;
  }
</style>
