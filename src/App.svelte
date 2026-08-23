<script lang="ts">
  import { onMount } from 'svelte';

  import TimeSeriesChart from './lib/charts/TimeSeriesChart.svelte';
  import BatterySelector from './lib/components/BatterySelector.svelte';
  import BatteryStateBadge from './lib/components/BatteryStateBadge.svelte';
  import EmptyState from './lib/components/EmptyState.svelte';
  import ExecutionContextNotice from './lib/components/ExecutionContextNotice.svelte';
  import MetricCard from './lib/components/MetricCard.svelte';
  import RecorderSettings from './lib/components/RecorderSettings.svelte';
  import { isMetricAvailable, type Metric } from './lib/domain/battery';
  import {
    dashboardScenarioCatalog,
    findDashboardScenario,
  } from './lib/fixtures/dashboard-scenarios';
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

  const defaultScenario =
    findDashboardScenario(dashboardScenarioCatalog.defaultScenarioId) ??
    dashboardScenarioCatalog.scenarios[0];

  if (!defaultScenario) {
    throw new Error('The simulated dashboard requires at least one scenario.');
  }

  let activeSection = $state<ProductSection>(defaultProductSection);
  let selectedScenarioId = $state(defaultScenario.id);
  let selectedBatteryId = $state(
    defaultScenario.selectedSnapshot?.id ?? 'all-batteries',
  );
  let liveDashboard = $state<BatteryDashboardData | null>(null);
  let isRefreshingLiveData = $state(false);
  const recorderClient = createDesktopRecorderClient();

  let scenario = $derived(findDashboardScenario(selectedScenarioId) ?? defaultScenario);
  let batteries = $derived(liveDashboard?.batteries ?? scenario.batteries);
  let aggregate = $derived(liveDashboard?.aggregate ?? scenario.aggregate);
  let isLiveData = $derived(liveDashboard !== null);
  let selectedSnapshot = $derived.by(() => {
    if (batteries.length === 0) return null;
    if (selectedBatteryId === 'all-batteries') return aggregate;

    return batteries.find((battery) => battery.id === selectedBatteryId) ?? null;
  });
  let batteryOptions = $derived.by(() => [
    ...(batteries.length > 1 ? [{ id: 'all-batteries', label: aggregate.label }] : []),
    ...batteries.map((battery) => ({
      id: battery.id,
      label: `${battery.label} (${battery.id})`,
    })),
  ]);
  let selectedSectionInfo = $derived(
    productSections.find((section) => section.id === activeSection) ??
      productSections[0],
  );
  let powerPoints = $derived(
    scenario.chart.map((point) => ({
      timestamp: point.timestamp,
      value: point.powerWatts,
    })),
  );
  let percentagePoints = $derived(
    scenario.chart.map((point) => ({
      timestamp: point.timestamp,
      value: point.percentage,
    })),
  );

  function selectScenario(id: string) {
    const nextScenario = findDashboardScenario(id);
    if (!nextScenario) return;

    selectedScenarioId = id;
    selectedBatteryId =
      nextScenario.batteries.length > 1
        ? 'all-batteries'
        : (nextScenario.batteries[0]?.id ?? 'all-batteries');
    activeSection = 'dashboard';
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

      liveDashboard = dashboard;
      selectedBatteryId =
        dashboard.batteries.length > 1
          ? 'all-batteries'
          : (dashboard.batteries[0]?.id ?? 'all-batteries');
    } catch {
      // The browser preview deliberately keeps its fixtures when Tauri is absent.
      liveDashboard = null;
    } finally {
      isRefreshingLiveData = false;
    }
  }

  onMount(() => {
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

  function formatTimestamp(timestamp: string | null): string {
    if (!timestamp) return 'No sample timestamp';

    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
      timeZone: 'UTC',
    }).format(new Date(timestamp));
  }

  function isStale(metric: Metric<number>): boolean {
    return metric.availability === 'stale';
  }

  const formatPower = (value: number) => `${formatNumber(value, 1, true)} W`;
  const formatPercentage = (value: number) => `${formatNumber(value, 0)}%`;
</script>

<svelte:head>
  <title
    >{isLiveData ? 'Battery Dashboard' : 'Battery Dashboard — Simulated preview'}</title
  >
  <meta
    name="description"
    content="A simulated preview of the local-first Battery Dashboard interface."
  />
</svelte:head>

<main class="dashboard-app">
  <aside class="sidebar">
    <div class="brand">
      <p class="eyebrow">Local-first Linux utility</p>
      <p class="brand__name">Battery Dashboard</p>
      <p class="brand__detail">
        {isLiveData ? 'Native local dashboard' : 'Browser simulated preview'}
      </p>
    </div>

    <SectionNavigation
      selectedSection={activeSection}
      onSelect={(section) => (activeSection = section)}
    />

    {#if !isLiveData}
      <label class="scenario-control">
        <span>Simulation scenario</span>
        <select
          value={selectedScenarioId}
          onchange={(event) => selectScenario(event.currentTarget.value)}
        >
          {#each dashboardScenarioCatalog.scenarios as option (option.id)}
            <option value={option.id}>{option.name}</option>
          {/each}
        </select>
      </label>
    {/if}

    <p class="sidebar__note">
      {isLiveData
        ? 'Live data and recorder controls stay local to this device.'
        : 'These values are fixtures only. No battery data is read or stored in this browser preview.'}
    </p>
  </aside>

  <section class="page-content" aria-labelledby="page-title">
    <header class="page-header">
      <div>
        <p class="eyebrow">{selectedSectionInfo.label}</p>
        <h1 id="page-title">{selectedSectionInfo.description}</h1>
      </div>
      <span class="preview-badge">{isLiveData ? 'Live data' : 'Simulated data'}</span>
    </header>

    <ExecutionContextNotice
      executionContext={isLiveData ? 'native-desktop' : 'simulated-preview'}
    />

    {#if activeSection === 'dashboard'}
      {#if selectedSnapshot}
        <section class="dashboard-hero" aria-label="Selected battery overview">
          <div class="dashboard-hero__charge">
            <p class="metric-label">Current charge</p>
            <p class="charge-value">
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
                onSelect={(id) => (selectedBatteryId = id)}
              />
            {/if}
            <p>
              {isLiveData
                ? 'Read locally from available Linux battery providers. Each metric names its source.'
                : scenario.description}
            </p>
          </div>
        </section>

        {#if selectedSnapshot.percentage.availability === 'stale'}
          <aside class="data-notice" aria-label="Stale sample warning">
            <strong>Last sample may be outdated.</strong>
            This scenario represents a suspend gap; the chart keeps the missing interval visible.
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
          <EmptyState
            title="Recent history chart arrives next"
            message="The optional local recorder can be managed in Settings. Stored samples will appear in the dashboard chart phase."
            hint="No sample is created until background recording is explicitly enabled."
          />
        {:else}
          <section class="chart-grid" aria-label="Simulated charts">
            <TimeSeriesChart
              id="power-chart"
              title="Battery power"
              description="Simulated charging and discharging observations from the last seven hours."
              points={powerPoints}
              formatValue={formatPower}
              color="var(--color-power)"
            />
            <TimeSeriesChart
              id="charge-chart"
              title="Charge level"
              description="A fixture preview of how a persisted charge trend will be displayed."
              points={percentagePoints}
              formatValue={formatPercentage}
              color="var(--color-accent)"
            />
          </section>
        {/if}
      {:else}
        <EmptyState
          title="No battery detected"
          message="This simulated scenario represents a desktop system or unsupported hardware."
          hint="The real application will show this state instead of inventing measurements."
        />
      {/if}
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
