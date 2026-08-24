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

<nav class="section-navigation" aria-label={label}>
  <ul>
    {#each productSections as section, index (section.id)}
      <li>
        <button
          type="button"
          aria-current={selectedSection === section.id ? 'page' : undefined}
          aria-label={`${section.label}: ${section.description}`}
          title={section.description}
          data-section={section.id}
          onclick={() => onSelect(section.id)}
        >
          <span class="section-navigation__index" aria-hidden="true"
            >{String(index + 1).padStart(2, '0')}</span
          >
          {section.label}
        </button>
      </li>
    {/each}
  </ul>
</nav>
