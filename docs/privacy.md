# Privacy

Battery Dashboard is designed as a local-only application.

## What it stores

When background recording is explicitly enabled, the application stores battery
observations locally in SQLite. Fields include UTC timestamps, Linux boot ID,
boot-relative time, battery state, percentage, energy, power, current, voltage,
temperature, capacity, and hardware cycle count when exposed by the device.
Each numeric field also stores its `upower`, `sysfs`, `derived`, or
`unavailable` provenance. It does not store raw battery serial numbers in the
SQLite schema or default exports.

The default database location is:

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

CSV and JSON exports are written only after the user supplies an absolute local
destination path. Existing files are never overwritten. Exports include the
selected records, schema metadata, time-zone, and unit information; they omit
raw serial and boot identifiers. Treat exported files as local device-usage
data and share them deliberately.

## Current status

Background collection happens only after an explicit Settings action enables
the user timer. The timer is disabled by default; it launches a one-shot
recorder every 60 seconds and exits. The app has no cloud endpoint, telemetry,
or network service.
