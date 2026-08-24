<script lang="ts">
  export type SessionState = 'charging' | 'discharging' | 'full' | 'unknown';
  export type SessionCompleteness = 'complete' | 'incomplete';
  export type SessionGapReason =
    | 'sampling gap'
    | 'computer suspended'
    | 'rebooted'
    | 'battery removed'
    | 'state changed'
    | 'invalid measurement'
    | string;

  export type SessionBattery = { id: string; label: string };

  /** Values are shown only when the query explicitly provides them. */
  export type BatterySession = {
    id: string;
    batteryId: string;
    batteryLabel?: string | null;
    state: SessionState;
    startedAt: Date | string | null;
    endedAt: Date | string | null;
    completeness: SessionCompleteness;
    gapReason?: SessionGapReason | null;
    durationMinutes?: number | null;
    startPercentage?: number | null;
    endPercentage?: number | null;
    percentageChange?: number | null;
    energyChangeWh?: number | null;
    averagePowerWatts?: number | null;
    peakPowerWatts?: number | null;
  };

  type Props = {
    id?: string;
    sessions?: readonly BatterySession[];
    batteries?: readonly SessionBattery[];
    selectedBatteryId?: string;
    selectedState?: SessionState | 'all';
    startDate?: string;
    endDate?: string;
    loading?: boolean;
    unsupportedReason?: string | null;
    onBatteryChange?: (batteryId: string) => void;
    onStateChange?: (state: SessionState | 'all') => void;
    onStartDateChange?: (date: string) => void;
    onEndDateChange?: (date: string) => void;
  };

  let {
    id = 'sessions',
    sessions = [],
    batteries = [],
    selectedBatteryId = 'all-batteries',
    selectedState = 'all',
    startDate = '',
    endDate = '',
    loading = false,
    unsupportedReason = null,
    onBatteryChange = () => {},
    onStateChange = () => {},
    onStartDateChange = () => {},
    onEndDateChange = () => {},
  }: Props = $props();

  const states: readonly (SessionState | 'all')[] = [
    'all',
    'charging',
    'discharging',
    'full',
    'unknown',
  ];

  function formatDate(value: Date | string | null): string | null {
    if (value === null) return null;
    const date = new Date(value);
    return Number.isFinite(date.getTime())
      ? new Intl.DateTimeFormat(undefined, {
          dateStyle: 'medium',
          timeStyle: 'short',
        }).format(date)
      : null;
  }

  function formatDuration(minutes: number): string {
    const wholeMinutes = Math.round(minutes);
    const hours = Math.floor(wholeMinutes / 60);
    const remainder = wholeMinutes % 60;
    return hours > 0 ? `${hours}h ${remainder}m` : `${remainder}m`;
  }

  function signed(value: number, suffix: string): string {
    return `${value > 0 ? '+' : ''}${value.toFixed(1)}${suffix}`;
  }

  function stateLabel(state: SessionState | 'all'): string {
    return state === 'all' ? 'All states' : state[0].toUpperCase() + state.slice(1);
  }

  const hasDuration = (
    session: BatterySession,
  ): session is BatterySession & {
    durationMinutes: number;
  } => session.durationMinutes !== null && session.durationMinutes !== undefined;
  const percentRange = (session: BatterySession) =>
    session.startPercentage !== null &&
    session.startPercentage !== undefined &&
    session.endPercentage !== null &&
    session.endPercentage !== undefined
      ? `${session.startPercentage.toFixed(0)}% → ${session.endPercentage.toFixed(0)}%`
      : 'Charge endpoints unavailable';

  let completeDischarges = $derived(
    sessions
      .filter(
        (session) =>
          session.state === 'discharging' && session.completeness === 'complete',
      )
      .filter(hasDuration),
  );
  let completeCharges = $derived(
    sessions
      .filter(
        (session) =>
          session.state === 'charging' && session.completeness === 'complete',
      )
      .filter(hasDuration),
  );
  let longestDischarge = $derived(
    [...completeDischarges].sort(
      (left, right) => right.durationMinutes - left.durationMinutes,
    )[0] ?? null,
  );
  let nearFullDischarge = $derived(
    completeDischarges.find((session) => (session.startPercentage ?? -1) >= 95) ?? null,
  );
  let chargeToFull = $derived(
    completeCharges.find((session) => (session.endPercentage ?? -1) >= 99) ?? null,
  );
</script>

<section class="sessions-view" aria-labelledby={`${id}-title`}>
  <header class="sessions-view__header">
    <div>
      <p class="sessions-view__eyebrow">Sessions</p>
      <h2 id={`${id}-title`}>Charging and discharging activity</h2>
      <p class="sessions-view__description">
        Complete and interrupted sessions are kept distinct. Statistics appear only when
        recorded.
      </p>
    </div>
  </header>

  <div class="sessions-view__filters" aria-label="Session filters">
    <label
      >Battery
      <select
        value={selectedBatteryId}
        onchange={(event) => onBatteryChange(event.currentTarget.value)}
      >
        <option value="all-batteries">All batteries</option>
        {#each batteries as battery (battery.id)}
          <option value={battery.id}>{battery.label}</option>
        {/each}
      </select>
    </label>
    <label
      >State
      <select
        value={selectedState}
        onchange={(event) =>
          onStateChange(event.currentTarget.value as SessionState | 'all')}
      >
        {#each states as state (state)}<option value={state}>{stateLabel(state)}</option
          >{/each}
      </select>
    </label>
    <label
      >From
      <input
        type="date"
        value={startDate}
        onchange={(event) => onStartDateChange(event.currentTarget.value)}
      />
    </label>
    <label
      >To
      <input
        type="date"
        value={endDate}
        onchange={(event) => onEndDateChange(event.currentTarget.value)}
      />
    </label>
  </div>

  {#if loading}
    <div class="sessions-view__placeholder" role="status">
      Loading recorded sessions…
    </div>
  {:else if unsupportedReason}
    <div class="sessions-view__placeholder" role="status">
      Session history is unavailable. {unsupportedReason}
    </div>
  {:else if sessions.length === 0}
    <div class="sessions-view__placeholder" role="status">
      No recorded sessions match these filters.
    </div>
  {:else}
    <section class="sessions-view__answers" aria-label="Recorded battery answers">
      <header>
        <p class="sessions-view__eyebrow">Observed answers</p>
        <h3>What your recorded runs show</h3>
      </header>
      <div>
        <article>
          <span>Longest on-battery run</span>
          <strong
            >{longestDischarge
              ? formatDuration(longestDischarge.durationMinutes)
              : 'Not measured yet'}</strong
          >
          <p>
            {longestDischarge
              ? percentRange(longestDischarge)
              : 'Requires one uninterrupted completed discharge session.'}
          </p>
        </article>
        <article>
          <span>From near-full charge</span>
          <strong
            >{nearFullDischarge
              ? formatDuration(nearFullDischarge.durationMinutes)
              : 'Not measured yet'}</strong
          >
          <p>
            {nearFullDischarge
              ? percentRange(nearFullDischarge)
              : 'Requires a recorded discharge that begins at 95% or more.'}
          </p>
        </article>
        <article>
          <span>Charge to full</span>
          <strong
            >{chargeToFull
              ? formatDuration(chargeToFull.durationMinutes)
              : 'Not measured yet'}</strong
          >
          <p>
            {chargeToFull
              ? percentRange(chargeToFull)
              : 'Requires a recorded charge run that reaches a full state.'}
          </p>
        </article>
      </div>
    </section>
    <ol class="sessions-view__list" aria-label="Recorded battery sessions">
      {#each sessions as session (session.id)}
        <li class="sessions-view__item">
          <div class="sessions-view__item-header">
            <div>
              <h3>{stateLabel(session.state)}</h3>
              {#if session.batteryLabel}<p>{session.batteryLabel}</p>{/if}
              {#if formatDate(session.startedAt) && formatDate(session.endedAt)}
                <p>{formatDate(session.startedAt)} – {formatDate(session.endedAt)}</p>
              {:else if formatDate(session.startedAt)}
                <p>Started {formatDate(session.startedAt)}</p>
              {:else}
                <p>Session time unavailable</p>
              {/if}
            </div>
            <span
              class:sessions-view__completeness--incomplete={session.completeness ===
                'incomplete'}
              class="sessions-view__completeness"
              >{session.completeness === 'complete' ? 'Complete' : 'Incomplete'}</span
            >
          </div>

          {#if session.completeness === 'incomplete'}
            <p class="sessions-view__gap">
              {session.gapReason
                ? `Interrupted: ${session.gapReason}.`
                : 'Interrupted; no reason was recorded.'}
            </p>
          {/if}

          {#if (session.durationMinutes !== null && session.durationMinutes !== undefined) || (session.percentageChange !== null && session.percentageChange !== undefined) || (session.energyChangeWh !== null && session.energyChangeWh !== undefined) || (session.averagePowerWatts !== null && session.averagePowerWatts !== undefined) || (session.peakPowerWatts !== null && session.peakPowerWatts !== undefined)}
            <dl class="sessions-view__metrics">
              {#if session.durationMinutes !== null && session.durationMinutes !== undefined}<div
                >
                  <dt>Duration</dt>
                  <dd>{formatDuration(session.durationMinutes)}</dd>
                </div>{/if}
              {#if session.percentageChange !== null && session.percentageChange !== undefined}<div
                >
                  <dt>Charge change</dt>
                  <dd>{signed(session.percentageChange, '%')}</dd>
                </div>{/if}
              {#if session.energyChangeWh !== null && session.energyChangeWh !== undefined}<div
                >
                  <dt>Energy change</dt>
                  <dd>{signed(session.energyChangeWh, ' Wh')}</dd>
                </div>{/if}
              {#if session.averagePowerWatts !== null && session.averagePowerWatts !== undefined}<div
                >
                  <dt>Average power</dt>
                  <dd>{session.averagePowerWatts.toFixed(1)} W</dd>
                </div>{/if}
              {#if session.peakPowerWatts !== null && session.peakPowerWatts !== undefined}<div
                >
                  <dt>Peak power</dt>
                  <dd>{session.peakPowerWatts.toFixed(1)} W</dd>
                </div>{/if}
            </dl>
          {/if}
        </li>
      {/each}
    </ol>
  {/if}
</section>

<style>
  .sessions-view {
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-card);
    padding: 1.25rem;
    background: var(--color-surface);
  }
  .sessions-view__eyebrow,
  h2,
  h3,
  p {
    margin: 0;
  }
  .sessions-view__eyebrow {
    color: var(--color-accent);
    font-size: 0.72rem;
    font-weight: 750;
    letter-spacing: 0.1em;
    text-transform: uppercase;
  }
  h2 {
    margin-top: 0.18rem;
    font-size: 1.05rem;
  }
  .sessions-view__description {
    max-width: 60ch;
    margin-top: 0.4rem;
    color: var(--color-text-secondary);
    font-size: 0.88rem;
    line-height: 1.45;
  }
  .sessions-view__filters {
    display: grid;
    grid-template-columns: repeat(4, minmax(8rem, 1fr));
    gap: 0.7rem;
    margin-top: 1.1rem;
  }
  .sessions-view__filters label {
    display: grid;
    gap: 0.3rem;
    color: var(--color-text-secondary);
    font-size: 0.78rem;
    font-weight: 700;
  }
  .sessions-view__filters select,
  .sessions-view__filters input {
    width: 100%;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.55rem;
    padding: 0.45rem 0.55rem;
    color: var(--color-text-primary);
    background: var(--color-surface-raised);
  }
  .sessions-view__placeholder {
    display: grid;
    min-height: 9rem;
    margin-top: 1rem;
    place-items: center;
    border: 1px dashed var(--color-border-subtle);
    border-radius: 0.75rem;
    padding: 1rem;
    color: var(--color-text-secondary);
    text-align: center;
  }
  .sessions-view__list {
    display: grid;
    gap: 0.7rem;
    margin: 1rem 0 0;
    padding: 0;
    list-style: none;
  }
  .sessions-view__answers {
    margin-top: 1rem;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.75rem;
    padding: 0.9rem;
    background: color-mix(in srgb, var(--color-surface-raised), transparent 25%);
  }
  .sessions-view__answers header {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
  }
  .sessions-view__answers h3 {
    margin: 0;
    font-size: 0.92rem;
  }
  .sessions-view__answers > div {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 0.55rem;
    margin-top: 0.75rem;
  }
  .sessions-view__answers article {
    min-width: 0;
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.6rem;
    padding: 0.7rem;
    background: var(--color-surface);
  }
  .sessions-view__answers span,
  .sessions-view__answers p {
    color: var(--color-text-secondary);
    font-size: 0.72rem;
    line-height: 1.35;
  }
  .sessions-view__answers strong {
    display: block;
    margin-top: 0.25rem;
    color: var(--color-text-primary);
    font-size: 0.9rem;
  }
  .sessions-view__answers p {
    margin: 0.35rem 0 0;
  }
  .sessions-view__item {
    border: 1px solid var(--color-border-subtle);
    border-radius: 0.75rem;
    padding: 0.9rem;
    background: var(--color-surface-raised);
  }
  .sessions-view__item-header {
    display: flex;
    gap: 0.7rem;
    justify-content: space-between;
  }
  h3 {
    font-size: 0.95rem;
  }
  .sessions-view__item-header p {
    margin-top: 0.22rem;
    color: var(--color-text-secondary);
    font-size: 0.8rem;
    line-height: 1.4;
  }
  .sessions-view__completeness {
    flex: none;
    align-self: start;
    border-radius: 999px;
    padding: 0.28rem 0.5rem;
    color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent), transparent 86%);
    font-size: 0.72rem;
    font-weight: 750;
  }
  .sessions-view__completeness--incomplete {
    color: var(--color-warning);
    background: color-mix(in srgb, var(--color-warning), transparent 86%);
  }
  .sessions-view__gap {
    margin-top: 0.7rem;
    border-left: 3px solid var(--color-warning);
    padding-left: 0.6rem;
    color: var(--color-text-secondary);
    font-size: 0.8rem;
    line-height: 1.4;
  }
  .sessions-view__metrics {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(7rem, 1fr));
    gap: 0.5rem;
    margin: 0.8rem 0 0;
  }
  .sessions-view__metrics div {
    padding: 0.5rem 0.6rem;
    border-radius: 0.55rem;
    background: color-mix(in srgb, var(--color-canvas), transparent 45%);
  }
  dt {
    color: var(--color-text-secondary);
    font-size: 0.7rem;
  }
  dd {
    margin: 0.15rem 0 0;
    font-size: 0.86rem;
    font-weight: 700;
  }
  @media (max-width: 42rem) {
    .sessions-view__filters {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  @media (max-width: 52rem) {
    .sessions-view__answers > div {
      grid-template-columns: 1fr;
    }
  }
</style>
