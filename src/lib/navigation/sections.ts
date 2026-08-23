/**
 * The stable, router-independent sections of the product.
 *
 * A union derived from this list keeps navigation state constrained to an
 * actual screen while letting the application choose how to render each one.
 */
export const productSections = [
  {
    id: 'dashboard',
    label: 'Dashboard',
    description: 'Current battery status and recent activity.',
  },
  {
    id: 'sessions',
    label: 'Sessions',
    description: 'Charging and discharging sessions.',
  },
  {
    id: 'history',
    label: 'History',
    description: 'Daily, weekly, and monthly battery history.',
  },
  {
    id: 'health',
    label: 'Health',
    description: 'Battery capacity, cycles, and degradation.',
  },
  {
    id: 'settings',
    label: 'Settings',
    description: 'Recording, data, and application preferences.',
  },
] as const;

export type ProductSection = (typeof productSections)[number]['id'];

export const defaultProductSection: ProductSection = 'dashboard';
