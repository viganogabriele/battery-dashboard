<script lang="ts">
  import { onMount } from 'svelte';
  import { SvelteDate } from 'svelte/reactivity';

  import BatterySelector from './lib/components/BatterySelector.svelte';
  import BatteryStateBadge from './lib/components/BatteryStateBadge.svelte';
  import CalendarHistoryView from './lib/components/CalendarHistoryView.svelte';
  import EmptyState from './lib/components/EmptyState.svelte';
  import ExportControls, {
    type ExportRequest,
  } from './lib/components/ExportControls.svelte';
  import HealthView from './lib/components/HealthView.svelte';
  import InsightsView, {
    type AnomalyReport,
  } from './lib/components/InsightsView.svelte';
  import MetricCard from './lib/components/MetricCard.svelte';
  import PowerProfileControls, {
    type PowerProfile,
    type PowerProfileState,
  } from './lib/components/PowerProfileControls.svelte';
  import RecentHistoryChart, {
    type RecentHistoryRecorderState,
  } from './lib/components/RecentHistoryChart.svelte';
  import RecorderSettings from './lib/components/RecorderSettings.svelte';
  import SessionsView from './lib/components/SessionsView.svelte';
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
    type BatterySessionHistoryData,
    type CalendarSummaryPeriod,
  } from './lib/services/session-history-client';

  type BatteryHealthResponseDto = {
    schemaVersion: 1;
    availability: 'available' | 'unavailable';
    unavailableReason: string | null;
    source: 'sqlite' | 'unavailable';
    batteryId: string | null;
    currentFullCapacityWh: number | null;
    currentFullCapacityRecordedAt: string | null;
    designCapacityWh: number | null;
    designCapacityRecordedAt: string | null;
    healthPercentage: number | null;
    healthRecordedAt: string | null;
    hardwareCycleCount: number | null;
    hardwareCycleCountRecordedAt: string | null;
    capacityHistory: readonly {
      recordedAt: string;
      fullCapacityWh: number;
    }[];
    trend: 'stable' | 'degrading' | 'noisy' | 'insufficient';
    trendSlopeWhPerDay: number | null;
    trendUpperConfidenceWhPerDay: number | null;
    trendInsufficiencyReason: string | null;
  };

  type ExportResponseDto = {
    schemaVersion: 1;
    availability: 'available' | 'unavailable';
    unavailableReason: string | null;
    dataType: string;
    format: string;
    destination: string | null;
    recordCount: number;
    bytesWritten: number | null;
    error: string | null;
  };

  type AnomalyResponseDto = AnomalyReport & {
    schemaVersion: 1;
    source: 'sqlite' | 'unavailable';
    batteryId: string | null;
  };

  type PowerProfileResponseDto = PowerProfileState & {
    schemaVersion: 1;
    requestedProfile: PowerProfile | null;
    changed: boolean;
  };

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
  let exportSelection = $state<ExportRequest | null>(null);
  let exportResult = $state<ExportResponseDto | null>(null);
  let isExporting = $state(false);
  let batteryHealth = $state<BatteryHealthResponseDto | null>(null);
  let isRefreshingHealth = $state(false);
  let anomalyReport = $state<AnomalyResponseDto | null>(null);
  let anomalyRangeHours = $state<24 | 168 | 720>(24);
  let isRefreshingAnomalies = $state(false);
  let powerProfile = $state<PowerProfileResponseDto | null>(null);
  let isRefreshingPowerProfile = $state(false);
  let isChangingPowerProfile = $state(false);

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
        reason: gap.reason.replaceAll('-', ' '),
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

  const quickLinks: readonly {
    section: ProductSection;
    label: string;
    description: string;
  }[] = [
    {
      section: 'chart',
      label: 'Live chart',
      description: 'Trace charge and gaps over time.',
    },
    {
      section: 'sessions',
      label: 'Sessions',
      description: 'Review charging and on-battery runs.',
    },
    {
      section: 'history',
      label: 'History',
      description: 'Compare daily, weekly, and monthly records.',
    },
    {
      section: 'health',
      label: 'Health',
      description: 'Check capacity, cycles, and wear signals.',
    },
    {
      section: 'insights',
      label: 'Insights',
      description: 'Review evidence-backed unusual battery behaviour.',
    },
    {
      section: 'export',
      label: 'Export',
      description: 'Choose a local data set and file format.',
    },
    {
      section: 'settings',
      label: 'Settings',
      description: 'Control opt-in background recording.',
    },
  ];

  function selectBattery(id: string) {
    selectedBatteryId = id;
    if (isLiveData) void refreshRecentHistory(id);
    if (isLiveData) void refreshSessionHistory(id);
    if (isLiveData && activeSection === 'health') void refreshBatteryHealth(id);
    if (isLiveData && activeSection === 'insights') void refreshAnomalies(id);
  }

  function selectSection(section: ProductSection) {
    activeSection = section;
    if (isLiveData && (section === 'sessions' || section === 'history')) {
      void refreshSessionHistory();
    }
    if (isLiveData && (section === 'dashboard' || section === 'chart')) {
      void refreshRecentHistory();
    }
    if (isLiveData && section === 'health') {
      void refreshBatteryHealth();
    }
    if (isLiveData && section === 'insights') {
      void refreshAnomalies();
    }
    if (isNativeRuntime && section === 'settings') {
      void refreshPowerProfile();
    }
  }

  function localDate(offsetDays = 0): string {
    const date = new SvelteDate();
    date.setDate(date.getDate() + offsetDays);
    return [
      date.getFullYear(),
      String(date.getMonth() + 1).padStart(2, '0'),
      String(date.getDate()).padStart(2, '0'),
    ].join('-');
  }

  function showSessionDay(offsetDays: number) {
    const date = localDate(offsetDays);
    sessionStartDate = date;
    sessionEndDate = date;
    void refreshSessionHistory();
  }

  function showHistoryDay(offsetDays: number) {
    const date = localDate(offsetDays);
    sessionStartDate = date;
    sessionEndDate = date;
    activeSection = 'history';
    void refreshSessionHistory();
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
      if (activeSection === 'health') await refreshBatteryHealth(selectedBatteryId);
      if (activeSection === 'insights') await refreshAnomalies(selectedBatteryId);
    } catch (error) {
      // Native failures must stay visible: fixtures belong only to browser preview.
      liveDashboard = null;
      recentHistory = null;
      sessionHistory = null;
      batteryHealth = null;
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

  async function refreshBatteryHealth(batteryId = selectedBatteryId) {
    if (isRefreshingHealth || !isNativeRuntime) return;

    isRefreshingHealth = true;
    batteryHealth = null;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      batteryHealth = await invoke<BatteryHealthResponseDto>('get_battery_health', {
        batteryId: batteryId === 'all-batteries' ? null : batteryId,
      });
    } catch {
      batteryHealth = null;
    } finally {
      isRefreshingHealth = false;
    }
  }

  async function refreshAnomalies(batteryId = selectedBatteryId) {
    if (isRefreshingAnomalies || !isNativeRuntime) return;

    isRefreshingAnomalies = true;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      anomalyReport = await invoke<AnomalyResponseDto>('get_battery_anomalies', {
        batteryId: batteryId === 'all-batteries' ? null : batteryId,
        rangeHours: anomalyRangeHours,
      });
    } catch {
      anomalyReport = null;
    } finally {
      isRefreshingAnomalies = false;
    }
  }

  function selectAnomalyRange(rangeHours: 24 | 168 | 720) {
    anomalyRangeHours = rangeHours;
    void refreshAnomalies();
  }

  async function refreshPowerProfile() {
    if (isRefreshingPowerProfile || !isNativeRuntime) return;

    isRefreshingPowerProfile = true;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      powerProfile = await invoke<PowerProfileResponseDto>('get_power_profile');
    } catch {
      powerProfile = null;
    } finally {
      isRefreshingPowerProfile = false;
    }
  }

  async function setPowerProfile(profile: PowerProfile) {
    if (isChangingPowerProfile || !isNativeRuntime) return;

    isChangingPowerProfile = true;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      powerProfile = await invoke<PowerProfileResponseDto>('set_power_profile', {
        profile,
      });
    } catch {
      powerProfile = null;
    } finally {
      isChangingPowerProfile = false;
    }
  }

  onMount(() => {
    isNativeRuntime = '__TAURI_INTERNALS__' in window;
    void refreshLiveData();
    const refreshInterval = window.setInterval(() => void refreshLiveData(), 15_000);

    return () => window.clearInterval(refreshInterval);
  });

  async function handleExport(request: ExportRequest) {
    exportSelection = request;
    exportResult = null;
    if (!isNativeRuntime) return;
    if (!request.destination) return;
    if (isExporting) return;

    isExporting = true;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      exportResult = await invoke<ExportResponseDto>('export_battery_history', {
        request: {
          dataType: request.dataType,
          format: request.format,
          destination: request.destination,
          batteryId: selectedBatteryId === 'all-batteries' ? null : selectedBatteryId,
          timezone: timezone(),
        },
      });
    } catch (error) {
      exportResult = {
        schemaVersion: 1,
        availability: 'unavailable',
        unavailableReason: 'command-failed',
        dataType: request.dataType,
        format: request.format,
        destination: request.destination,
        recordCount: 0,
        bytesWritten: null,
        error: error instanceof Error ? error.message : String(error),
      };
    } finally {
      isExporting = false;
    }
  }

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
      <div class="brand__mark" aria-hidden="true">⌁</div>
      <div>
        <p class="brand__name">Battery Dashboard</p>
        <p class="brand__detail">
          {isNativeRuntime ? 'Local desktop' : 'Desktop app required'}
        </p>
      </div>
    </div>

    <SectionNavigation selectedSection={activeSection} onSelect={selectSection} />

    <div class="sidebar__footer">
      <span class:status-pill--live={isLiveData} class="status-pill" role="status">
        <span class="status-pill__dot" aria-hidden="true"></span>
        {isLiveData
          ? 'Live data'
          : isNativeRuntime
            ? 'Waiting for battery'
            : 'Desktop only'}
      </span>
      <p class="sidebar__version">Local data stays on this device.</p>
    </div>
  </aside>

  <section class="page-content" aria-labelledby="page-title">
    <header class="page-header">
      <div>
        <p class="eyebrow">{selectedSectionInfo.label}</p>
        <h1 id="page-title">{selectedSectionInfo.label}</h1>
        <p class="page-header__description">{selectedSectionInfo.description}</p>
      </div>
      <div class="page-header__status" aria-live="polite">
        {#if isRefreshingLiveData}
          <span class="status-chip status-chip--pending">Refreshing</span>
        {:else if isLiveData}
          <span class="status-chip status-chip--live">Live</span>
        {:else if isNativeRuntime}
          <span class="status-chip">No battery</span>
        {:else}
          <span class="status-chip">Desktop only</span>
        {/if}
      </div>
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
              <p class="charge-unavailable">Charge percentage is not exposed.</p>
            {/if}
            <p class="sample-time">
              Updated {formatTimestamp(selectedSnapshot.updatedAt)}
            </p>
          </div>

          <div class="dashboard-hero__state">
            <div class="hero-state__heading">
              <span class="eyebrow">Power state</span>
              <BatteryStateBadge state={selectedSnapshot.state} />
            </div>
            {#if batteryOptions.length > 1}
              <BatterySelector
                batteries={batteryOptions}
                selectedId={selectedBatteryId}
                onSelect={selectBattery}
              />
            {/if}
            <p class="hero-state__source">
              Read from Linux battery providers. Metrics stay unavailable when the
              provider does not expose them.
            </p>
          </div>
        </section>

        {#if selectedSnapshot.percentage.availability === 'stale'}
          <aside class="sample-warning" aria-label="Stale sample warning">
            <strong>Sample may be outdated.</strong>
            The provider marked this reading stale after a delayed update or resume.
          </aside>
        {/if}

        <section class="dashboard-section" aria-labelledby="dashboard-chart-title">
          <div class="section-heading">
            <div>
              <p class="eyebrow">Recent activity</p>
              <h2 id="dashboard-chart-title">Live charge chart</h2>
            </div>
            <button
              class="text-button"
              type="button"
              onclick={() => selectSection('chart')}
            >
              Open full chart <span aria-hidden="true">→</span>
            </button>
          </div>
          <RecentHistoryChart
            points={recentHistoryPoints}
            gaps={recentHistoryGaps}
            summary={recentHistorySummary}
            loading={isRefreshingHistory}
            recorderState={recentHistoryRecorderState}
            selectedRange={historyRange}
            onRangeChange={selectHistoryRange}
          />
        </section>

        <section class="quick-links" aria-labelledby="quick-links-title">
          <div class="section-heading">
            <div>
              <p class="eyebrow">Workspace</p>
              <h2 id="quick-links-title">Battery records</h2>
            </div>
          </div>
          <div class="quick-links__grid">
            {#each quickLinks as link (link.section)}
              <button
                class="quick-link"
                type="button"
                onclick={() => selectSection(link.section)}
                aria-label={`${link.label}: ${link.description}`}
              >
                <span class="quick-link__label">{link.label}</span>
                <span class="quick-link__description">{link.description}</span>
                <span class="quick-link__arrow" aria-hidden="true">↗</span>
              </button>
            {/each}
          </div>
        </section>

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
      {:else}
        <EmptyState
          title={isNativeRuntime
            ? 'No Linux battery detected'
            : 'Open the desktop application'}
          message={isNativeRuntime
            ? (liveDataError ??
              'UPower and sysfs did not return a battery for this device.')
            : 'A browser tab cannot read UPower, sysfs, or local SQLite history.'}
          hint={isNativeRuntime
            ? 'Connect a supported battery or check that Linux exposes one under /sys/class/power_supply.'
            : 'Run the Battery Dashboard desktop application to view real local readings.'}
        />
      {/if}
    {:else if activeSection === 'chart'}
      {#if isLiveData}
        <section class="focus-section" aria-label="Live charge history">
          <RecentHistoryChart
            points={recentHistoryPoints}
            gaps={recentHistoryGaps}
            summary={recentHistorySummary}
            loading={isRefreshingHistory}
            recorderState={recentHistoryRecorderState}
            selectedRange={historyRange}
            onRangeChange={selectHistoryRange}
          />
        </section>
      {:else}
        <EmptyState
          title={isNativeRuntime
            ? 'Live chart is waiting for a battery'
            : 'Open the desktop app for the live chart'}
          message={isNativeRuntime
            ? (liveDataError ?? 'No current battery provider is reporting chart data.')
            : 'The chart is built from local readings and is not available in a browser tab.'}
          hint={isNativeRuntime
            ? 'Enable recording in Settings to keep persistent readings between sessions.'
            : 'Launch the desktop application to read local battery history.'}
        />
      {/if}
    {:else if activeSection === 'sessions'}
      <div class="session-actions">
        <div class="session-actions__presets" aria-label="Session date shortcuts">
          <button type="button" onclick={() => showSessionDay(0)}>Today</button>
          <button type="button" onclick={() => showSessionDay(-1)}>Yesterday</button>
          <button
            type="button"
            onclick={() => {
              sessionStartDate = '';
              sessionEndDate = '';
              void refreshSessionHistory();
            }}>All recorded</button
          >
        </div>
        <button type="button" onclick={rebuildSessions} disabled={isRebuildingSessions}>
          {isRebuildingSessions ? 'Rebuilding sessions…' : 'Rebuild sessions'}
        </button>
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
      <div class="session-actions">
        <div class="session-actions__presets" aria-label="History date shortcuts">
          <button type="button" onclick={() => showHistoryDay(0)}>Today</button>
          <button type="button" onclick={() => showHistoryDay(-1)}>Yesterday</button>
          <button
            type="button"
            onclick={() => {
              sessionStartDate = '';
              sessionEndDate = '';
              void refreshSessionHistory();
            }}>All recorded</button
          >
        </div>
      </div>
      <CalendarHistoryView
        periods={(sessionHistory?.[calendarPeriod] ?? []).map((item) => ({
          id: `${item.bucket}-${item.batteryId ?? 'all'}`,
          label: item.bucket,
          observedSamples: item.observedSamples,
          minimumPercentage: item.minimumPercentage,
          maximumPercentage: item.maximumPercentage,
          observedEnergyWh: item.observedEnergyUsedWh ?? item.observedEnergyChargedWh,
          recordedDurationSeconds: item.coverageSeconds,
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
    {:else if activeSection === 'health'}
      {#if isLiveData && selectedSnapshot}
        {#if batteryHealth?.availability === 'unavailable'}
          <EmptyState
            title={batteryHealth.unavailableReason === 'multiple-batteries'
              ? 'Select one battery for health'
              : 'No recorded health history'}
            message={batteryHealth.unavailableReason === 'multiple-batteries'
              ? 'Health analysis keeps capacity values separate for each physical battery.'
              : 'Health reports use recorded full-capacity and design-capacity samples only.'}
            hint={batteryHealth.unavailableReason === 'recorder-disabled'
              ? 'Enable recording in Settings, then return after a few samples are collected.'
              : 'Enable recording in Settings to build a local capacity history.'}
          />
        {:else}
          <HealthView
            currentFullCapacityWh={batteryHealth?.availability === 'available'
              ? batteryHealth.currentFullCapacityWh
              : isMetricAvailable(selectedSnapshot.energyFullWh)
                ? selectedSnapshot.energyFullWh.value
                : null}
            designCapacityWh={batteryHealth?.availability === 'available'
              ? batteryHealth.designCapacityWh
              : isMetricAvailable(selectedSnapshot.energyDesignWh)
                ? selectedSnapshot.energyDesignWh.value
                : null}
            hardwareCycleCount={batteryHealth?.availability === 'available'
              ? batteryHealth.hardwareCycleCount
              : isMetricAvailable(selectedSnapshot.cycleCount)
                ? selectedSnapshot.cycleCount.value
                : null}
            capacityHistory={batteryHealth?.availability === 'available'
              ? batteryHealth.capacityHistory.map((point) => ({
                  timestamp: point.recordedAt,
                  fullCapacityWh: point.fullCapacityWh,
                }))
              : []}
            trend={batteryHealth?.availability === 'available'
              ? batteryHealth.trend
              : 'insufficient'}
          />
        {/if}
      {:else}
        <EmptyState
          title={isNativeRuntime
            ? 'Battery health needs a local battery'
            : 'Open the desktop app for health'}
          message={isNativeRuntime
            ? (liveDataError ??
              'Capacity and cycle values are unavailable without a battery provider.')
            : 'Health calculations use local capacity and cycle readings from the desktop app.'}
          hint={isNativeRuntime
            ? 'Select a physical battery from the dashboard when Linux exposes one.'
            : 'Launch the desktop application to inspect capacity and wear data.'}
        />
      {/if}
    {:else if activeSection === 'insights'}
      <InsightsView
        report={anomalyReport}
        loading={isRefreshingAnomalies}
        rangeHours={anomalyRangeHours}
        onRangeChange={selectAnomalyRange}
        onRefresh={() => void refreshAnomalies()}
      />
    {:else if activeSection === 'export'}
      <section class="focus-section" aria-label="Export local battery data">
        <ExportControls onExport={handleExport} />
        {#if isExporting}
          <p class="action-feedback" role="status">
            Writing the selected local history…
          </p>
        {:else if exportResult}
          <p
            class:action-feedback--error={exportResult.availability !== 'available'}
            class="action-feedback"
            role="status"
          >
            {#if exportResult.availability === 'available'}
              Wrote {exportResult.recordCount}
              {exportResult.dataType.replaceAll('-', ' ')}
              {exportResult.format.toUpperCase()} record{exportResult.recordCount === 1
                ? ''
                : 's'}
              to {exportResult.destination}.
            {:else}
              Export could not be written: {exportResult.error ??
                exportResult.unavailableReason ??
                'unknown error'}
            {/if}
          </p>
        {:else if exportSelection && !isNativeRuntime}
          <p class="action-feedback" role="status">
            Export is available only in the desktop application, where local history can
            be written to the path you choose.
          </p>
        {:else if exportSelection && !exportSelection.destination}
          <p class="action-feedback" role="status">
            Enter an absolute destination path before exporting.
          </p>
        {/if}
      </section>
    {:else if activeSection === 'settings'}
      <section class="settings-panel">
        <RecorderSettings client={recorderClient} />
        <PowerProfileControls
          state={powerProfile}
          loading={isRefreshingPowerProfile}
          changing={isChangingPowerProfile}
          onRefresh={() => void refreshPowerProfile()}
          onSelect={setPowerProfile}
        />
      </section>
    {/if}
  </section>
</main>
