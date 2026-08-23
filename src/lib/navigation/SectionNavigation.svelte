<script lang="ts">
  import { productSections, type ProductSection } from './sections';

  type Props = {
    selectedSection: ProductSection;
    onSelect?: (section: ProductSection) => void;
    label?: string;
  };

  // The parent owns state; callback props are the idiomatic Svelte 5 event API.
  let {
    selectedSection,
    onSelect = () => undefined,
    label = 'Primary navigation',
  }: Props = $props();
</script>

<nav aria-label={label}>
  <ul>
    {#each productSections as section (section.id)}
      <li>
        <button
          type="button"
          aria-current={selectedSection === section.id ? 'page' : undefined}
          aria-label={`${section.label}: ${section.description}`}
          onclick={() => onSelect(section.id)}
        >
          {section.label}
        </button>
      </li>
    {/each}
  </ul>
</nav>
