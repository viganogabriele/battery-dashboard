# Battery Dashboard — Product and Development Plan

## 1. Document status

This document is the authoritative implementation plan for Battery Dashboard.
It captures the agreed product scope, architecture, portability strategy,
development phases, quality gates, privacy constraints, and known hardware
limitations.

Phases 1–4 are complete: the repository foundation, simulated Svelte dashboard,
Tauri desktop shell, and live UPower/sysfs reader are implemented.
Implementation continues phase by phase. Advanced features must not be
implemented before the core version is stable.

## 2. Product vision

Battery Dashboard is a modern native Linux desktop application for monitoring
laptop batteries over time. It must show live metrics, preserve historical
samples while the window is closed, explain missing or unreliable data, and
remain useful on machines with one or more batteries.

The initial officially supported platform is Arch Linux and Arch-based
distributions. The application architecture must remain Linux-generic so that
other distributions can be supported without rewriting the battery, storage,
or user-interface layers.

Omarchy is an optional integration target, not a runtime dependency and not
part of the core product identity. A separate Omarchy plugin may later expose
compact consumption information in the Omarchy bar and open the desktop app.

## 3. Platform support strategy

### 3.1 Version 1 support

Version 1 will officially target:

- Arch Linux and Arch derivatives;
- a systemd user session for background recording;
- UPower when available;
- the Linux power-supply sysfs interface as a direct source and fallback;
- a desktop environment or Wayland compositor capable of running Tauri's
  WebKitGTK-based window.

The application must not assume Omarchy, Hyprland, GNOME, KDE, or a particular
status bar.

### 3.2 Broader Linux compatibility

The live application and database layers should work on other Linux
distributions when their WebKitGTK, UPower, sysfs, and system libraries are
compatible.

Background recording will be accessed through a `SchedulerBackend` boundary:

- `SystemdUserScheduler` is the only implementation required for version 1;
- non-systemd systems may still use the live dashboard;
- OpenRC, runit, cron, or desktop-autostart adapters can be evaluated after
  version 1 without changing the recorder or database;
- the UI must report "background recording is unsupported on this system"
  instead of pretending that history is being collected.

Support claims must be based on tested distributions. "Linux-compatible" must
not be used to imply that every battery firmware exposes every metric.

### 3.3 Optional Omarchy integration

Omarchy support will be an optional post-version-1 integration:

- no Omarchy files are required by the core app;
- the core app never edits `/usr/share/omarchy`;
- a plugin, if created, lives in a separate integration directory or repository;
- the plugin may read the same SQLite database or call a stable read-only CLI;
- the plugin may show percentage, state, and live power draw;
- clicking it may launch Battery Dashboard;
- installing or removing the plugin must not affect the core database;
- the desktop app itself must never create a tray or top-bar icon.

## 4. Core product requirements

The completed version 1 must provide:

- current percentage and battery state;
- charge/discharge power in watts;
- voltage, current, and battery temperature when available;
- remaining runtime or time-to-full estimates only when supported by adequate
  source data;
- charge and discharge charts;
- usage and charging sessions;
- daily, weekly, and monthly history;
- health, current maximum capacity, design capacity, and hardware cycle count;
- capacity degradation trends when enough historical evidence exists;
- CSV and JSON export;
- support for BAT0, BAT1, differently named batteries, and multiple batteries;
- explicit handling of missing fields, suspend gaps, clock changes, reboot, and
  battery removal;
- a background recorder that runs while the UI is closed and can be disabled.

## 5. Explicit non-goals for version 1

Version 1 will not include:

- cloud synchronization;
- user accounts;
- telemetry or analytics;
- an always-running HTTP server;
- Electron;
- a system tray or top-bar item;
- privileged installation or runtime helpers;
- notifications, anomaly detection, automatic profile switching, or
  per-process energy attribution;
- fabricated values for metrics that the hardware does not expose.

## 6. Technology stack

- Tauri 2 for the native desktop window and frontend/backend IPC;
- Svelte 5 for the interface;
- TypeScript for frontend application code and data contracts;
- Vite for development and static production builds;
- Tailwind CSS plus application-owned design tokens;
- Rust for battery providers, normalization, persistence, queries, exports,
  scheduler management, and the recorder;
- SQLite through a Rust library, without depending on the `sqlite3` CLI;
- a systemd user service and timer for one-shot recording every 60 seconds.

Stable dependency versions will be selected and locked when Phase 1 is
implemented. Major-version upgrades are separate tasks and must not be mixed
into feature phases.

## 7. High-level architecture

```text
                              APPLICATION OPEN

  UPower system D-Bus ----+
                          +--> Rust battery providers --> Tauri commands
  /sys/class/power_supply +              |                     |
                                         |                     v
                                         +--------------> Svelte UI
                                         |
                                         +--------------> SQLite reads


                         APPLICATION OPEN OR CLOSED

  systemd --user timer --> Rust recorder (one-shot) --> SQLite write --> exit
          every 60 s
```

There is no frontend server in production. Vite's server exists only during
development. Tauri loads compiled static assets in the production window.

The desktop application does not write periodic samples. The recorder is the
single owner of scheduled persistence, preventing duplicate records while the
window is open. If recording is disabled, the app can still show live data and
an in-memory short chart, but it must not silently persist samples.

## 8. Planned repository layout

```text
battery-dashboard/
├── crates/
│   └── battery-core/            # Shared platform-neutral domain types
├── src/
│   ├── lib/
│   │   ├── components/
│   │   ├── charts/
│   │   ├── stores/
│   │   ├── services/
│   │   ├── fixtures/
│   │   └── types/
│   ├── App.svelte
│   └── app.css
├── src-tauri/
│   ├── capabilities/
│   ├── migrations/
│   ├── src/
│   │   ├── battery/
│   │   ├── storage/
│   │   ├── sessions/
│   │   ├── export/
│   │   ├── scheduler/
│   │   ├── bin/recorder.rs
│   │   ├── lib.rs
│   │   └── main.rs
│   ├── Cargo.toml
│   └── tauri.conf.json
├── systemd/
│   ├── battery-dashboard-recorder.service
│   └── battery-dashboard-recorder.timer
├── integrations/
│   └── omarchy/                 # Optional and added only after core v1
├── tests/
│   └── fixtures/sysfs/
├── docs/
│   ├── architecture.md
│   ├── data-model.md
│   ├── hardware-support.md
│   └── privacy.md
├── DEVELOPMENT_PLAN.md
└── README.md
```

This layout is directional. Phase 1 may start smaller, and modules should be
split only when they acquire a clear responsibility.

## 9. Backend boundaries

### 9.1 Battery providers

Rust will define a testable battery-provider interface. Initial providers:

- `UPowerProvider`: discovers power-supply batteries through the UPower system
  D-Bus API and reads standardized properties;
- `SysfsProvider`: enumerates `/sys/class/power_supply/*` by device type rather
  than assuming BAT0;
- `CompositeProvider`: merges the two at field level and records provenance.

Provider roots and clients must be injectable so tests can use filesystem and
D-Bus fixtures without depending on the developer's laptop.

### 9.2 Field precedence

The exact precedence may vary by field, but the general rule is:

1. use a valid standardized UPower value;
2. supplement or replace it with a valid direct sysfs value when UPower does
   not expose the field or reports it as unknown;
3. derive a value only from compatible source units and only when the formula
   is physically valid;
4. otherwise return the field as unavailable.

Every metric carries its source and last-update timestamp.

### 9.3 Unit normalization

The internal model uses:

- watts for power;
- watt-hours for energy;
- volts for voltage;
- amperes for current;
- degrees Celsius for temperature;
- a 0–100 floating-point value for percentage.

`charge_*` values in ampere-hours and `energy_*` values in watt-hours must never
be combined as if they were the same unit. Charge can be converted to energy
only when a valid corresponding voltage is available and the conversion is
explicitly marked as derived.

Power precedence:

1. `power_now` or UPower energy rate;
2. absolute current multiplied by voltage;
3. unavailable.

Charge/discharge direction is normalized from battery state because firmware
sign conventions are inconsistent. Invalid, sentinel, negative-capacity, and
out-of-range values are rejected rather than clamped silently.

### 9.4 Missing and stale data

Missing data is represented as `null`/`Option`, never zero. The frontend model
will expose at least:

```ts
type MetricSource = "upower" | "sysfs" | "derived" | "unavailable";

type Metric<T> = {
  value: T | null;
  source: MetricSource;
  updatedAt: string | null;
};
```

The UI must distinguish:

- unsupported by hardware;
- temporarily unavailable;
- stale after a collection gap;
- invalid source data;
- still collecting enough history for an estimate.

A generic ACPI thermal-zone temperature must not be labelled as battery
temperature unless the zone can be reliably associated with the battery.

## 10. Multiple-battery behavior

Each physical battery remains a first-class device in storage and UI. Users can
select an individual battery or an aggregate "All batteries" view.

Aggregate rules:

- current energy, full energy, design energy, and compatible power values are
  summed;
- percentage is weighted by current full capacity, not averaged arithmetically;
- health is total full energy divided by total design energy for devices that
  provide both values;
- voltage, current, and temperature remain per-device values;
- aggregate state is `charging`, `discharging`, `full`, `idle`, `mixed`, or
  `unknown` according to the contributing devices;
- missing contributions are disclosed in the UI;
- aggregate runtime is shown only when the energy and net-power inputs are
  sufficiently complete and coherent.

A stable local device identifier will be derived from available native path and
hardware metadata. Raw serial numbers are not needed in normal exports and must
be excluded by default.

## 11. Runtime estimates

Estimate priority:

1. a positive UPower time-to-empty or time-to-full value consistent with the
   current state;
2. a local estimate based on recent observed history;
3. unavailable.

A local discharge estimate requires, at minimum:

- ten valid observations;
- at least ten minutes of covered time;
- no reboot or large collection gap in the window;
- a coherent discharging state;
- valid remaining energy and a positive robust average power draw.

Charging estimates require equivalent evidence plus a valid energy deficit.
The UI must label the estimate source and show "collecting data" or "not
available" when confidence requirements are not met.

No values are interpolated across suspend, reboot, battery removal, or long
sampling gaps.

## 12. Time, suspend, and reboot handling

Every sample stores:

- UTC wall-clock timestamp;
- Linux boot ID;
- boot-relative time when available;
- collection source and completeness flags.

Local time is used only for display and calendar grouping. This prevents stored
history from becoming ambiguous around timezone or daylight-saving changes.

The recorder does not reconstruct missed samples. After resume it records the
current state, and charts render the missing interval as a gap. Session logic
ends or marks a session incomplete when the interval exceeds the configured
tolerance. A changed boot ID always creates a boundary.

## 13. SQLite design

The database lives under:

```text
${XDG_DATA_HOME:-~/.local/share}/battery-dashboard/battery.sqlite3
```

SQLite uses versioned migrations, WAL mode, foreign keys, a busy timeout, and
short transactions.

Initial logical tables:

### `batteries`

- internal ID;
- stable local ID;
- current native path;
- vendor/model/technology when available;
- first-seen and last-seen timestamps;
- presence and metadata-quality fields.

### `samples`

- battery ID;
- UTC timestamp, boot ID, and boot-relative time;
- state and AC status;
- percentage;
- energy now, full, and full-design;
- signed normalized power and current;
- voltage and temperature;
- hardware cycle count;
- source and completeness flags.

### `sessions`

- battery ID or explicit aggregate scope;
- charge, discharge, full/idle, or unknown type;
- observed start/end;
- start/end percentage and energy;
- observed duration;
- transferred energy;
- average/peak power;
- completion and boundary reason.

### `daily_summaries`

- local calendar day and timezone identifier;
- battery ID;
- observed energy use and charge;
- min/max percentage;
- representative full capacity and health;
- coverage/quality values.

### `schema_migrations`

- migration version and applied timestamp.

The database has no automatic destructive retention policy in version 1.

## 14. Recorder and scheduler lifecycle

The recorder is a small Rust binary that performs one transaction and exits.
It must not become a custom daemon.

The primary no-privilege installation locations are:

```text
~/.local/libexec/battery-dashboard/recorder
~/.config/systemd/user/battery-dashboard-recorder.service
~/.config/systemd/user/battery-dashboard-recorder.timer
```

Behavior:

- background recording is disabled until the user explicitly enables it;
- enabling installs or atomically refreshes the recorder and user units;
- the user timer triggers once per minute;
- disabling stops and disables the timer but preserves the database;
- uninstall removes application-owned executables and units;
- historical data is preserved by default and removed only by an explicit
  purge action;
- all service management uses `systemctl --user`;
- neither the app nor its scripts invoke `sudo` or `pkexec`.

The systemd service should use safe hardening compatible with reading sysfs,
using the system D-Bus, and writing only to the application data directory.

An AppImage cannot be treated as a permanent recorder path because its mount
disappears when the app exits. Enabling recording must therefore copy the
version-matched recorder to the stable per-user libexec path.

## 15. Tauri security and desktop behavior

- one normal desktop window;
- no tray icon or status notifier;
- no remote content;
- no production localhost server;
- a restrictive content-security policy;
- explicit Tauri 2 capabilities instead of broad defaults;
- no generic shell execution permission exposed to the webview;
- no generic home-directory filesystem access exposed to the webview;
- all database and export operations implemented by typed Rust commands;
- save dialogs scoped to user-initiated export actions;
- closing the window terminates the desktop process.

The recorder remains independent because it is managed by the user timer, not
because the GUI hides in the background.

## 16. User interface plan

Primary navigation:

1. Dashboard
2. Sessions
3. History
4. Health
5. Settings

### Dashboard

- selected battery or aggregate selector;
- percentage and state hero card;
- power, voltage, current, temperature, and estimate cards;
- source/availability details;
- charge/discharge chart for 2 h, 6 h, 12 h, and 24 h;
- min, max, average, and observed energy statistics;
- visible recorder-disabled or data-stale status.

### Sessions

- charge and discharge session list;
- duration, percentage change, energy change, average/peak power;
- complete/incomplete status and gap reason;
- per-battery filters.

### History

- daily, weekly, and monthly ranges;
- percentage, energy, power, and coverage charts;
- local-calendar aggregation with DST-safe queries;
- data table alternative to charts.

### Health

- current maximum and design capacity;
- health percentage when calculable;
- hardware cycle count or an explicit unsupported message;
- capacity-over-time chart;
- conservative degradation trend after sufficient history.

### Settings

- enable/disable background recording;
- recorder health and last successful sample;
- database path and size;
- export controls;
- source diagnostics;
- privacy and hardware limitations.

The initial charts should use accessible application-owned SVG components.
Rust query endpoints will downsample long ranges before sending them to the UI.
Charts must have keyboard-accessible summaries or table equivalents.

## 17. Health and degradation rules

Health is computed only when both current full capacity and design capacity are
valid:

```text
health_percent = full_capacity / design_capacity * 100
```

The original raw values remain available for diagnostics. Plausible values over
100% are not automatically forced to 100%, because conservative design ratings
can produce them.

Hardware cycle count is displayed only when UPower or sysfs exposes it. The app
must not substitute plug/unplug events or tracked partial discharges for the
firmware cycle count. A future "tracked equivalent cycles" metric, if added,
must have a separate name and definition.

Degradation uses representative daily maximum full-capacity observations to
avoid treating partial charge as capacity loss. A trend is withheld unless
there are enough distinct days, enough calendar span, and acceptable data
coverage. Projection to a threshold is withheld when slope direction or
statistical quality is inconclusive.

## 18. Sessions and history rules

Session types:

- charging;
- discharging;
- full/idle on external power;
- unknown or mixed.

Session boundaries include:

- state transition;
- AC transition when relevant;
- battery disappearance;
- reboot;
- a sampling gap greater than the accepted tolerance;
- incompatible or invalid measurements.

Session generation must be incremental and idempotent. A rebuild operation may
recompute derived sessions from raw samples after algorithm or schema changes.
It must never alter raw samples.

Daily, weekly, and monthly history is derived from UTC samples using the user's
selected or current IANA timezone. Coverage must be shown so a day with ten
minutes of observations is not presented as equivalent to a fully recorded day.

## 19. Export design

CSV and JSON export will support:

- raw samples;
- sessions;
- daily/weekly/monthly summaries;
- selected date range;
- selected batteries or aggregate data.

Exports include schema version, generation time, timezone, and units. Null
values remain null/empty and are not converted to zero. Raw battery serials are
excluded by default. Files are written atomically to a user-selected path.

JSON should preserve typed values and metadata. CSV should use stable column
ordering and standards-compliant quoting.

## 20. Theme and branding

The core application uses a neutral Battery Dashboard identity. It must not look
or behave like an Omarchy-only utility.

The UI uses application-owned semantic tokens for background, surface, border,
text, accent, charging, discharging, warning, and error colors. It supports a
polished dark theme first and follows system light/dark preference where
reliable.

An optional read-only Omarchy theme adapter may be explored later. Failure to
locate or parse an Omarchy theme must always fall back to the built-in theme and
must never modify Omarchy configuration.

## 21. Installation, packaging, and uninstallation

### Primary development and early-user path

- build from source;
- per-user installation under `~/.local`;
- per-user desktop entry under `~/.local/share/applications`;
- no privilege escalation.

### Arch Linux distribution

- validate a reproducible release build;
- provide checksums and documented build dependencies;
- evaluate an AUR package after the per-user installer is stable;
- package the desktop binary, recorder, desktop entry, icons, and systemd user
  unit templates without automatically enabling recording.

### Broader Linux distribution

- AppImage can be offered for the desktop application;
- the recorder must still be installed into a stable per-user path;
- distribution packages are added only after testing their native dependency
  versions;
- support documentation must list tested distributions and known limitations.

Uninstallation removes application files and disables/removes the user timer.
The database is kept by default. A separate explicit purge command removes
historical data after confirmation.

## 22. Privacy model

- all measurements remain on the local machine;
- no network service is started;
- no cloud, login, telemetry, crash upload, or analytics;
- no sudo or pkexec;
- exports occur only after a user action;
- recording is opt-in and can be disabled at any time;
- the database location and size are visible in Settings;
- uninstall documentation explains that history is preserved by default;
- raw serial numbers are not exported by default;
- future notifications and anomaly detection must remain local.

## 23. Development phases

### Phase 1 — Project structure and initial documentation — Complete

Deliverables:

- scaffold Svelte 5, TypeScript, Vite, and Tailwind CSS;
- initialize frontend and Rust formatting/linting configuration;
- create the minimal directory structure;
- add README, architecture, privacy, testing, and hardware-support documents;
- document Arch development dependencies and XDG locations;
- add package scripts and locked dependencies;
- do not implement real battery access yet.

Exit tests:

- dependency installation;
- TypeScript check;
- frontend format/lint/build;
- minimal Rust format/build/test.

### Phase 2 — Svelte interface with simulated data — Complete

Deliverables:

- application shell and all primary navigation destinations;
- dashboard metric cards and initial responsive SVG chart;
- frontend domain types;
- simulated single- and multi-battery sources;
- charging, discharging, full, missing-data, stale-data, no-battery, suspend-gap,
  and error scenarios;
- a development-only fixture selector;
- loading, empty, unsupported, and error states;
- keyboard and screen-reader basics.

Exit tests:

- formatter and unit conversion tests;
- aggregate multi-battery tests;
- component tests;
- responsive and accessibility smoke checks;
- TypeScript check and production frontend build.

### Phase 3 — Tauri desktop application and visual theme — Complete

Deliverables:

- initialize Tauri 2 around the static Svelte SPA;
- configure one normal desktop window;
- exclude tray behavior;
- ensure closing the window exits the application;
- configure restrictive capabilities and CSP;
- implement the first complete visual theme and semantic tokens;
- verify production operation without Vite or an HTTP server;
- retain simulated data.

Exit tests:

- Tauri development launch;
- Tauri debug build;
- no tray/top-bar icon;
- process-exit verification;
- offline static-bundle smoke test;
- frontend quality suite.

### Phase 4 — Real UPower and sysfs data — Complete

Deliverables:

- provider interface, UPower provider, sysfs provider, and composite provider;
- enumeration of all system power-supply batteries;
- unit normalization and source provenance;
- current, voltage, power, temperature, energy, capacity, health inputs, state,
  cycle count, and UPower estimate inputs;
- typed Tauri commands for live snapshots and diagnostics;
- graceful UPower-unavailable and sysfs-incomplete paths;
- live UI refresh using real data.

Exit tests:

- sysfs fixture tests for energy-based and charge-based hardware;
- UPower mock tests;
- BAT0, BAT1, unusual names, and multiple batteries;
- missing, invalid, and sentinel values;
- derived-power tests;
- real Arch hardware smoke test;
- Rustfmt, Clippy with warnings denied, and Rust tests.

### Phase 5 — SQLite and periodic Rust recorder — Complete

Deliverables:

- versioned SQLite schema and migrations;
- WAL, foreign keys, indexes, busy timeout, and integrity checks;
- one-shot Rust recorder using the same provider code as the app;
- systemd user service/timer templates;
- safe per-user enable, disable, status, and update operations;
- Settings toggle and recorder status;
- boot ID, timestamps, gap metadata, and duplicate prevention;
- recorder disabled by default;
- no-sudo installation into stable XDG/user paths.

Exit tests:

- empty and upgraded database migrations;
- concurrent reader/writer tests;
- duplicate and transaction rollback tests;
- manual recorder execution;
- systemd unit verification;
- enable/disable/re-enable checks;
- journal and last-success verification;
- simulated suspend/reboot gaps;
- SQLite integrity check;
- full frontend and Rust quality gates.

This completes the technical MVP.

### Phase 6 — Main dashboard and recent-history chart — Complete

Deliverables:

- complete live dashboard;
- selected-battery and aggregate views;
- two-, six-, twelve-, and twenty-four-hour charts;
- charge/discharge colors and real gap rendering;
- source, freshness, min/max/average, and observed-energy details;
- Rust range queries and downsampling;
- database history merged with clearly transient in-memory live points;
- explicit recorder-disabled behavior.

Exit tests:

- range and downsampling tests;
- chart tests for charging, discharging, mixed, and missing data;
- empty/stale/disabled-recorder cases;
- end-to-end simulated flow;
- real-hardware dashboard smoke test;
- accessibility and responsive verification.

This completes the usable MVP.

### Phase 7 — Sessions and calendar history — Complete

Deliverables:

- incremental, idempotent session detection;
- charge/discharge/full/unknown sessions;
- reboot, suspend, gap, battery-removal, and state-change boundaries;
- session statistics and completeness labels;
- daily, weekly, and monthly aggregation;
- timezone- and DST-aware grouping;
- filters by battery, state, and date;
- session rebuild operation from immutable raw samples.

Exit tests:

- table-driven session state transitions;
- interrupted and incomplete sessions;
- reboot and suspend scenarios;
- multiple batteries with different states;
- DST and timezone cases;
- idempotent rebuild;
- history query and UI tests.

This completes the beta.

### Phase 8 — Health, cycles, degradation, export, and release docs

Deliverables:

- current full and design capacity;
- health calculation with availability rules;
- hardware cycle count and unsupported state;
- capacity-over-time chart;
- conservative daily degradation trend;
- insufficient-data and inconclusive-trend states;
- raw, session, and summary CSV/JSON exports;
- atomic export and privacy-safe defaults;
- per-user installer and uninstaller;
- desktop launcher without tray integration;
- complete installation, uninstallation, privacy, database, hardware-limit, and
  troubleshooting documentation;
- release build validation on Arch Linux.

Exit tests:

- health input matrix;
- synthetic stable, degrading, noisy, and insufficient series;
- CSV escaping and stable schema;
- JSON round trip;
- install/update/disable/uninstall/purge tests in an isolated user environment;
- complete frontend and Rust test suites;
- Tauri release build and real-hardware smoke test.

This completes version 1.0.

### Phase 9 — Advanced features, only after version 1

#### Local notifications

- opt-in thresholds for percentage and genuine battery temperature;
- deduplication and cooldown;
- local desktop notifications without a tray process.

#### Anomaly detection

- local-only historical baseline;
- unusual draw, unexpectedly fast discharge, or interrupted charge detection;
- transparent explanations and confidence;
- no remote models or uploads.

#### Energy profiles

- detect a supported profile backend such as power-profiles-daemon;
- show the active profile;
- change it only after an explicit user action;
- no privilege escalation and a clear unsupported state.

#### Per-process impact

- begin with a technical feasibility spike;
- use only interfaces already readable by the unprivileged user;
- never change RAPL or kernel permissions automatically;
- never label CPU-time heuristics as measured watts;
- present estimates as "activity impact" with methodology and confidence;
- omit the feature when reliable data is unavailable.

### Phase 10 — Optional Omarchy plugin

This phase is optional and may start only after the version-1 data contract and
database schema are stable.

Possible deliverables:

- a separate Omarchy bar plugin;
- compact percentage, state, and power display;
- a short read-only graph or summary panel;
- click action to launch Battery Dashboard;
- shared read-only access through a documented CLI or database query API;
- independent install, enable, disable, and removal flow;
- no duplication of the core recorder;
- no requirement for the plugin to use the full desktop application.

The plugin must follow Omarchy's user-plugin conventions and must never modify
packaged files under `/usr/share/omarchy`.

## 24. Release milestones

| Milestone | Included phases | Definition |
| --- | ---: | --- |
| UI prototype | 1–3 | Simulated interface in a real Tauri window |
| Technical MVP | 1–5 | Real data, SQLite, and optional 60-second recording |
| Usable MVP | 1–6 | Useful dashboard and recent-history chart |
| Beta | 1–7 | Sessions and calendar history |
| Version 1.0 | 1–8 | All core product requirements and release documentation |
| Advanced release | 9 | Opt-in advanced features |
| Omarchy integration | 10 | Optional bar plugin, independent from the app |

## 25. Quality gates

Every applicable phase must run:

```text
pnpm format:check
pnpm lint
pnpm check
pnpm test
pnpm build

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Additional gates are added when relevant:

- Tauri debug and release builds;
- browser-based UI smoke and accessibility checks;
- systemd unit verification and lifecycle checks;
- SQLite migration, concurrency, and integrity tests;
- simulated provider fixtures;
- real-hardware checks on available Arch systems;
- packaging install/update/uninstall tests.

Tests must not rely exclusively on the development machine's battery. Real
hardware verification supplements deterministic fixtures; it does not replace
them.

## 26. Required simulation matrix

The test fixtures must cover:

- one energy-based battery;
- one charge-based battery;
- BAT0 and BAT1 together;
- batteries with non-BAT names;
- charging, discharging, full, idle, mixed, and unknown states;
- AC online and offline;
- missing temperature;
- missing cycle count;
- missing power but valid current and voltage;
- missing UPower with valid sysfs;
- UPower values newer or older than sysfs values;
- invalid, overflow, sentinel, and out-of-range values;
- battery removal and reinsertion;
- suspend gap;
- reboot boundary;
- wall-clock correction;
- daylight-saving transition;
- empty database and long history;
- recorder disabled;
- read-only or temporarily locked database;
- no battery present.

## 27. Phase completion report

After every phase, development must provide a checkpoint and then continue to
the next phase unless a real blocker or product-changing decision requires user
input.

Required format:

```text
Phase N completed

Implemented:
- ...

Files created or modified:
- path: purpose

Tests executed:
- command -> result

Real-hardware verification:
- detected source/hardware
- available and unavailable fields

Known limitations or decisions:
- ...
```

## 28. Known limitations to document

- firmware may omit temperature, cycle count, current, or full-design capacity;
- UPower estimates may be absent or unstable;
- sysfs naming and available properties vary by driver;
- aggregate voltage/current has no universally meaningful single value;
- history before recording is enabled cannot be reconstructed;
- suspend and shutdown produce intentional gaps;
- per-process energy use is generally not directly attributable without
  additional kernel/hardware facilities or permissions;
- non-systemd background scheduling is not part of version 1;
- AppImage packaging does not by itself provide a stable executable path for a
  recorder that must run after the AppImage closes;
- broader Linux support depends on tested WebKitGTK and system-library versions.

## 29. Reference projects and documentation

These sources are architectural and UX references only. Their code must not be
copied without a deliberate license and implementation review.

- Omarchy Battery Usage:
  <https://github.com/OverStyleFR/omarchy-battery-usage>
- GNOME Power Statistics:
  <https://github.com/GNOME/gnome-power-manager>
- UPower Device API:
  <https://upower.freedesktop.org/docs/Device.html>
- Linux power-supply class:
  <https://docs.kernel.org/power/power_supply_class.html>
- BatteryScope:
  <https://github.com/ptcodes/BatteryScope>
- Jolt:
  <https://github.com/jordond/jolt>
- Tauri frontend guidance:
  <https://v2.tauri.app/start/frontend/>
- Tauri security capabilities:
  <https://v2.tauri.app/security/capabilities/>
- systemd timers:
  <https://www.freedesktop.org/software/systemd/man/latest/systemd.timer.html>
- SQLite write-ahead logging:
  <https://sqlite.org/wal.html>

## 30. Decisions intentionally deferred

The following decisions are not required for planning and must be resolved at
the relevant implementation or release phase:

- final public product name and icon;
- reverse-domain Tauri bundle identifier;
- public repository owner/namespace;
- open-source license;
- exact AUR package name;
- which non-Arch distributions become officially tested targets;
- whether the optional Omarchy integration is kept in this repository or moved
  to a dedicated plugin repository.
