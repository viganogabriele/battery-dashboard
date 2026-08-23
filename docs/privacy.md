# Privacy

Battery Dashboard is designed as a local-only application.

## What it will store

When background recording is explicitly enabled, the application stores battery
observations locally in SQLite. Fields include UTC timestamps, Linux boot ID,
boot-relative time, battery state, percentage, energy, power, current, voltage,
temperature, capacity, and hardware cycle count when exposed by the device.
Each numeric field also stores its `upower`, `sysfs`, `derived`, or
`unavailable` provenance. It may later store non-sensitive device metadata
needed to recognise a local battery over time.

The planned default database location is:

```text
${XDG_DATA_HOME:-~/.local/share}/battery-dashboard/battery.sqlite3
```

## What it will not do

- No cloud synchronization, account, login, telemetry, analytics, or crash
  upload.
- No always-running HTTP server or remote content.
- No `sudo`, `pkexec`, or privileged helper.
- No background recording until the user enables it.
- No export unless the user initiates it.
- No raw battery serial numbers in default exports.

## Recording control and removal

The recorder runs from an opt-in `systemd --user` timer and can be disabled at
any time. Enabling it explicitly copies the recorder to a user-owned XDG data
location and writes two user unit files below XDG config; it never uses `sudo`,
`pkexec`, or a system-wide unit. Disabling stops future sampling but retains
existing history. Uninstall will also preserve the local database by default;
an explicit purge action will be required to delete history.

## Exports

Future CSV and JSON exports will be written only to a user-selected location.
They will include the selected measurements, schema metadata, time-zone and
unit information. Users should treat exported files as local device-usage data
and share them deliberately.

## Current status

During Phase 5, the Tauri command still reads current UPower and sysfs values
only locally. Background collection happens only after an explicit Settings
action enables the user timer. The timer is disabled by default; it launches a
one-shot recorder every 60 seconds and exits. The app has no cloud endpoint,
telemetry, export, or network service.
