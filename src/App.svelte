<script lang="ts">
  import { onMount } from 'svelte';

  import BatterySelector from './lib/components/BatterySelector.svelte';
  import BatteryStateBadge from './lib/components/BatteryStateBadge.svelte';
  import EmptyState from './lib/components/EmptyState.svelte';
  import MetricCard from './lib/components/MetricCard.svelte';
  import RecentHistoryChart, {
    type RecentHistoryRecorderState,
  } from './lib/components/RecentHistoryChart.svelte';
  import RecorderSettings from './lib/components/RecorderSettings.svelte';
  import SessionsView from './lib/components/SessionsView.svelte';
  import CalendarHistoryView from './lib/components/CalendarHistoryView.svelte';
  import { isMetricAvailable, type Metric } from './lib/domain/battery';
  import SectionNavigation from './lib/navigation/SectionNavigation.svelte';
  import {
    defaultProductSection,
    productSections,
    type ProductSection,
  } from './lib/navigation/sections';
  import {
    createBatteryDashboardClient,
    type BatteryDashboardData,
    type BatteryDashboardResponseDto,
  } from './lib/services/battery-dashboard-client';
  import { createDesktopRecorderClient } from './lib/services/recorder-client';
  import {
    createDesktopRecentBatteryHistoryClient,
    type RecentBatteryHistoryData,
    type RecentHistoryRangeHours,
  } from './lib/services/recent-history-client';
  import {
    createDesktopSessionHistoryClient,
    type CalendarSummaryPeriod,
    type BatterySessionHistoryData,
  } from './lib/services/session-history-client';

  let activeSection = $state<ProductSection>(defaultProductSection);
  let selectedBatteryId = $state('all-batteries');
  let liveDashboard = $state<BatteryDashboardData | null>(null);
  let isNativeRuntime = $state(false);
  let liveDataError = $state<string | null>(null);
  let isRefreshingLiveData = $state(false);
  let recentHistory = $state<RecentBatteryHistoryData | null>(null);
  let isRefreshingHistory = $state(false);
  let historyRange = $state<RecentHistoryRangeHours>(24);
  const recorderClient = createDesktopRecorderClient();
  const recentHistoryClient = createDesktopRecentBatteryHistoryClient();
  const sessionHistoryClient = createDesktopSessionHistoryClient();
  let sessionHistory = $state<BatterySessionHistoryData | null>(null);
  let isRefreshingSessions = $state(false);
  let isRebuildingSessions = $state(false);
  let sessionStateFilter = $state<
    'all' | 'charging' | 'discharging' | 'full' | 'unknown'
  >('all');
  let calendarPeriod = $state<CalendarSummaryPeriod>('daily');
  let sessionStartDate = $state('');
  let sessionEndDate = $state('');

  let batteries = $derived(liveDashboard?.batteries ?? []);
  let aggregate = $derived(liveDashboard?.aggregate ?? null);
  let isLiveData = $derived(liveDashboard !== null);
  let selectedSnapshot = $derived.by(() => {
    if (batteries.length === 0) return null;
    if (selectedBatteryId === 'all-batteries') return aggregate;

    return batteries.find((battery) => battery.id === selectedBatteryId) ?? null;
  });
  let batteryOptions = $derived.by(() => [
    ...(batteries.length > 1 && aggregate
      ? [{ id: 'all-batteries', label: aggregate.label }]
      : []),
    ...batteries.map((battery) => ({
      id: battery.id,
      label: `${battery.label} (${battery.id})`,
    })),
  ]);
  let selectedSectionInfo = $derived(
    productSections.find((section) => section.id === activeSection) ??
      productSections[0],
  );
  let recentHistoryPoints = $derived(
    recentHistory?.points.map((point) => ({
      timestamp: point.recordedAt,
      percentage: point.percentage.value,
      state: point.state,
      persisted: point.kind === 'persisted',
    })) ?? [],
  );
  let recentHistoryGaps = $derived(
    recentHistory?.gaps
      .filter((gap) => gap.endsAt !== null)
      .map((gap) => ({
        start: gap.startsAt,
        end: gap.endsAt ?? gap.startsAt,
        reason: gap.reason.replace('-', ' '),
      })) ?? [],
  );
  let recentHistorySummary = $derived(
    recentHistory
      ? {
          minimumPercentage: recentHistory.summary.percentage.minimum,
          maximumPercentage: recentHistory.summary.percentage.maximum,
          averagePercentage: recentHistory.summary.percentage.average,
          observedEnergyWh: recentHistory.summary.observedEnergyWh.change,
        }
      : null,
  );
  let recentHistoryRecorderState = $derived.by<RecentHistoryRecorderState>(() => {
    if (recentHistory?.unavailableReason === 'recorder-disabled') return 'disabled';
    if (recentHistory?.unavailableReason === 'unsupported') return 'unsupported';
    if (recentHistory?.unavailableReason === 'database-unavailable') return 'error';
    return 'enabled';
  });

  function selectBattery(id: string) {
    selectedBatteryId = id;
    if (isLiveData) void refreshRecentHistory(id);
    if (isLiveData) void refreshSessionHistory(id);
  }

  function timezone(): string {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  }
  async function refreshSessionHistory(batteryId = selectedBatteryId) {
    if (isRefreshingSessions) return;
    isRefreshingSessions = true;
    try {
      sessionHistory = await sessionHistoryClient.getHistory({
        batteryId: batteryId === 'all-batteries' ? undefined : batteryId,
        states: sessionStateFilter === 'all' ? undefined : [sessionStateFilter],
        startDate: sessionStartDate || undefined,
        endDate: sessionEndDate || undefined,
        timezone: timezone(),
      });
    } catch {
      sessionHistory = null;
    } finally {
      isRefreshingSessions = false;
    }
  }
  function refreshSessionFilters() {
    if (isLiveData) void refreshSessionHistory();
  }
  async function rebuildSessions() {
    if (isRebuildingSessions) return;
    isRebuildingSessions = true;
    try {
      await sessionHistoryClient.rebuild();
      await refreshSessionHistory();
    } finally {
      isRebuildingSessions = false;
    }
  }

  function selectHistoryRange(range: RecentHistoryRangeHours) {
    historyRange = range;
    if (isLiveData) void refreshRecentHistory();
  }

  async function refreshLiveData() {
    if (isRefreshingLiveData) return;

    isRefreshingLiveData = true;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const client = createBatteryDashboardClient(() =>
        invoke<BatteryDashboardResponseDto>('get_battery_dashboard'),
      );
      const dashboard = await client.getDashboard();

      isNativeRuntime = true;
      liveDashboard = dashboard;
      liveDataError = null;
      const suggestedBatteryId =
        dashboard.batteries.length > 1
          ? 'all-batteries'
          : (dashboard.batteries[0]?.id ?? 'all-batteries');
      const selectionIsStillAvailable =
        selectedBatteryId === 'all-batteries'
          ? dashboard.batteries.length > 1
          : dashboard.batteries.some((battery) => battery.id === selectedBatteryId);
      selectedBatteryId = selectionIsStillAvailable
        ? selectedBatteryId
        : suggestedBatteryId;
      await refreshRecentHistory(selectedBatteryId);
      await refreshSessionHistory(selectedBatteryId);
    } catch (error) {
      // Native failures must stay visible: fixtures belong only to browser preview.
      liveDashboard = null;
      recentHistory = null;
      sessionHistory = null;
      liveDataError = `Could not read local battery data: ${error instanceof Error ? error.message : String(error)}`;
    } finally {
      isRefreshingLiveData = false;
    }
  }

  async function refreshRecentHistory(batteryId = selectedBatteryId) {
    if (isRefreshingHistory) return;

    isRefreshingHistory = true;
    try {
      recentHistory = await recentHistoryClient.getRecentHistory({
        batteryId: batteryId === 'all-batteries' ? undefined : batteryId,
        rangeHours: historyRange,
        maxPoints: Math.min(historyRange * 60, 720),
      });
    } catch {
      recentHistory = null;
    } finally {
      isRefreshingHistory = false;
    }
  }

  onMount(() => {
    isNativeRuntime = '__TAURI_INTERNALS__' in window;
    void refreshLiveData();
    const refreshInterval = window.setInterval(() => void refreshLiveData(), 15_000);

    return () => window.clearInterval(refreshInterval);
  });

  function formatNumber(value: number, digits = 1, sign = false): string {
    return new Intl.NumberFormat(undefined, {
      maximumFractionDigits: digits,
      minimumFractionDigits: digits,
      signDisplay: sign ? 'always' : 'auto',
    }).format(value);
  }

  function formatMetric(
    metric: Metric<number>,
    unit: string,
    digits = 1,
    sign = false,
  ): string | null {
    return isMetricAvailable(metric)
      ? `${formatNumber(metric.value, digits, sign)}${unit === '%' ? '' : ' '}${unit}`
      : null;
  }

  function formatDuration(metric: Metric<number>): string | null {
    if (!isMetricAvailable(metric)) return null;

    const hours = Math.floor(metric.value / 60);
    const minutes = Math.round(metric.value % 60);
    return hours > 0 ? `${hours} h ${minutes} min` : `${minutes} min`;
  }

  function sessionViewState(
    state: 'charging' | 'discharging' | 'full' | 'idle' | 'unknown',
  ): 'charging' | 'discharging' | 'full' | 'unknown' {
    return state === 'idle' ? 'unknown' : state;
  }

  function formatTimestamp(timestamp: string | null): string {
    if (!timestamp) return 'No sample timestamp';

    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(timestamp));
  }

  function isStale(metric: Metric<number>): boolean {
    return metric.availability === 'stale';
  }
</script>

<svelte:head>
  <title>Battery Dashboard</title>
  <meta name="description" content="A local-first Linux battery dashboard." />
</svelte:head>

<main class="dashboard-app">
  <aside class="sidebar">
    <div class="brand">
      <p class="eyebrow">Local-first Linux utility</p>
      <p class="brand__name">Battery Dashboard</p>
      <p class="brand__detail">
        {isNativeRuntime ? 'Native local dashboard' : 'Desktop app required'}
      </p>
    </div>

    <SectionNavigation
      selectedSection={activeSection}
      onSelect={(section) => (activeSection = section)}
    />

    <p class="sidebar__note">
      {isLiveData
        ? 'Live data and recorder controls stay local to this device.'
        : 'This dashboard never substitutes simulated readings for local battery data.'}
    </p>
  </aside>

  <section class="page-content" aria-labelledby="page-title">
    <header class="page-header">
      <div>
        <p class="eyebrow">{selectedSectionInfo.label}</p>
        <h1 id="page-title">{selectedSectionInfo.description}</h1>
      </div>
      <span class="preview-badge">{isLiveData ? 'Live data' : 'Unavailable'}</span>
    </header>

    {#if activeSection === 'dashboard'}
      {#if selectedSnapshot}
        <section class="dashboard-hero" aria-label="Selected battery overview">
          <div class="dashboard-hero__charge">
            <p class="metric-label">Current charge</p>
            <p
              class="charge-value"
              class:charge-value--unavailable={!isMetricAvailable(
                selectedSnapshot.percentage,
              )}
            >
              {formatMetric(selectedSnapshot.percentage, '%', 0) ?? 'Unavailable'}
            </p>
            {#if isMetricAvailable(selectedSnapshot.percentage)}
              <meter
                min="0"
                max="100"
                value={selectedSnapshot.percentage.value}
                aria-label={`Battery charge: ${formatMetric(selectedSnapshot.percentage, '%', 0)}`}
              ></meter>
            {:else}
              <p class="charge-unavailable">Charge percentage is unavailable.</p>
            {/if}
            <p class="sample-time">
              Sample: {formatTimestamp(selectedSnapshot.updatedAt)}
            </p>
          </div>

          <div class="dashboard-hero__state">
            <BatteryStateBadge state={selectedSnapshot.state} />
            {#if batteryOptions.length > 1}
              <BatterySelector
                batteries={batteryOptions}
                selectedId={selectedBatteryId}
                onSelect={selectBattery}
              />
            {/if}
            <p>
              {isLiveData
                ? 'Read locally from available Linux battery providers. Each metric names its source.'
                : 'Local battery data is shown only when a provider reports it.'}
            </p>
          </div>
        </section>

        {#if selectedSnapshot.percentage.availability === 'stale'}
          <aside class="data-notice" aria-label="Stale sample warning">
            <strong>Last sample may be outdated.</strong>
            The provider marked this battery reading as stale; history keeps gaps visible.
          </aside>
        {/if}

        <section class="metric-grid" aria-label="Battery metrics">
          <MetricCard
            label="Battery power"
            value={formatMetric(selectedSnapshot.powerWatts, 'W', 1, true)}
            source={selectedSnapshot.powerWatts.source}
            stale={isStale(selectedSnapshot.powerWatts)}
          />
          <MetricCard
            label="Voltage"
            value={formatMetric(selectedSnapshot.voltageVolts, 'V', 2)}
            source={selectedSnapshot.voltageVolts.source}
            stale={isStale(selectedSnapshot.voltageVolts)}
          />
          <MetricCard
            label="Current"
            value={formatMetric(selectedSnapshot.currentAmps, 'A', 2, true)}
            source={selectedSnapshot.currentAmps.source}
            stale={isStale(selectedSnapshot.currentAmps)}
          />
          <MetricCard
            label="Temperature"
            value={formatMetric(selectedSnapshot.temperatureCelsius, '°C', 1)}
            source={selectedSnapshot.temperatureCelsius.source}
            stale={isStale(selectedSnapshot.temperatureCelsius)}
          />
          <MetricCard
            label={selectedSnapshot.state === 'charging'
              ? 'Time to full'
              : 'Runtime estimate'}
            value={formatDuration(selectedSnapshot.timeRemainingMinutes)}
            source={selectedSnapshot.timeRemainingMinutes.source}
            stale={isStale(selectedSnapshot.timeRemainingMinutes)}
            unavailableLabel="Insufficient data"
          />
          <MetricCard
            label="Cycle count"
            value={selectedSnapshot.cycleCount.value}
            source={selectedSnapshot.cycleCount.source}
            stale={isStale(selectedSnapshot.cycleCount)}
            unavailableLabel="Not exposed"
          />
        </section>

        {#if isLiveData}
          <RecentHistoryChart
            points={recentHistoryPoints}
            gaps={recentHistoryGaps}
            summary={recentHistorySummary}
            loading={isRefreshingHistory}
            recorderState={recentHistoryRecorderState}
            selectedRange={historyRange}
            onRangeChange={selectHistoryRange}
          />
        {/if}
      {:else}
        <EmptyState
          title={isNativeRuntime
            ? 'Local battery data unavailable'
            : 'Open the desktop application'}
          message={isNativeRuntime
            ? (liveDataError ?? 'Waiting for local battery providers.')
            : 'This browser page cannot access UPower, sysfs, or your local SQLite history.'}
          hint={isNativeRuntime
            ? 'Check that UPower is running and that Linux exposes a battery under /sys/class/power_supply.'
            : 'Run pnpm tauri dev from the project directory to see real local readings.'}
        />
      {/if}
    {:else if activeSection === 'sessions'}
      <div class="session-actions">
        <button type="button" onclick={rebuildSessions} disabled={isRebuildingSessions}>
          {isRebuildingSessions ? 'Rebuilding sessions…' : 'Rebuild local sessions'}
        </button>
        <p>Rebuild uses immutable local samples and does not collect new data.</p>
      </div>
      <SessionsView
        sessions={(sessionHistory?.sessions ?? []).map((session) => ({
          id: session.id,
          batteryId: session.batteryId ?? 'unknown',
          batteryLabel: session.batteryId,
          state: sessionViewState(session.state),
          startedAt: session.startedAt,
          endedAt: session.endedAt,
          completeness: session.completeness === 'complete' ? 'complete' : 'incomplete',
          gapReason: session.boundaryReason.replaceAll('-', ' '),
          durationMinutes:
            session.durationSeconds === null ? null : session.durationSeconds / 60,
          percentageChange:
            session.startPercentage === null || session.endPercentage === null
              ? null
              : session.endPercentage - session.startPercentage,
          energyChangeWh: session.transferredEnergyWh,
          averagePowerWatts: session.averagePowerWatts,
          peakPowerWatts: session.peakPowerWatts,
        }))}
        batteries={batteryOptions.filter((battery) => battery.id !== 'all-batteries')}
        {selectedBatteryId}
        selectedState={sessionStateFilter}
        startDate={sessionStartDate}
        endDate={sessionEndDate}
        loading={isRefreshingSessions}
        unsupportedReason={sessionHistory?.availability === 'unavailable'
          ? sessionHistory.unavailableReason
          : null}
        onBatteryChange={selectBattery}
        onStateChange={(state) => {
          sessionStateFilter = state;
          refreshSessionFilters();
        }}
        onStartDateChange={(date) => {
          sessionStartDate = date;
          refreshSessionFilters();
        }}
        onEndDateChange={(date) => {
          sessionEndDate = date;
          refreshSessionFilters();
        }}
      />
    {:else if activeSection === 'history'}
      <CalendarHistoryView
        periods={(sessionHistory?.[calendarPeriod] ?? []).map((item) => ({
          id: `${item.bucket}-${item.batteryId ?? 'all'}`,
          label: item.bucket,
          coveragePercent:
            item.coverageRatio === null ? null : item.coverageRatio * 100,
          minimumPercentage: item.minimumPercentage,
          maximumPercentage: item.maximumPercentage,
          observedEnergyWh: item.observedEnergyUsedWh ?? item.observedEnergyChargedWh,
          averagePowerWatts: null,
        }))}
        batteries={batteryOptions.filter((battery) => battery.id !== 'all-batteries')}
        selectedAggregation={calendarPeriod}
        {selectedBatteryId}
        selectedState={sessionStateFilter}
        startDate={sessionStartDate}
        endDate={sessionEndDate}
        loading={isRefreshingSessions}
        unsupportedReason={sessionHistory?.availability === 'unavailable'
          ? sessionHistory.unavailableReason
          : null}
        onAggregationChange={(period) => (calendarPeriod = period)}
        onBatteryChange={selectBattery}
        onStateChange={(state) => {
          sessionStateFilter = state;
          refreshSessionFilters();
        }}
        onStartDateChange={(date) => {
          sessionStartDate = date;
          refreshSessionFilters();
        }}
        onEndDateChange={(date) => {
          sessionEndDate = date;
          refreshSessionFilters();
        }}
      />
    {:else if activeSection === 'settings'}
      <section class="settings-panel">
        <RecorderSettings client={recorderClient} />
      </section>
    {:else}
      <section class="planned-panel">
        <p class="eyebrow">Planned screen</p>
        <EmptyState
          title={`${selectedSectionInfo.label} arrives in a later phase`}
          message={selectedSectionInfo.description}
          hint="The navigation is active now so the dashboard structure can be tested before real data is connected."
        />
      </section>
    {/if}
  </section>
</main>
