# Privacy

Battery Dashboard is designed as a local-only application.

## What it will store

When background recording is explicitly enabled, the application will store
battery observations locally in SQLite. Planned fields include timestamps,
battery state, percentage, energy, power, current, voltage, temperature,
capacity, and hardware cycle count when exposed by the device. It may also
store non-sensitive device metadata needed to recognise a local battery over
time.

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

The planned recorder runs from a `systemd --user` timer and can be disabled at
any time. Disabling it will stop future sampling but retain existing history.
Uninstall will also preserve the local database by default; an explicit purge
action will be required to delete history.

## Exports

Future CSV and JSON exports will be written only to a user-selected location.
They will include the selected measurements, schema metadata, time-zone and
unit information. Users should treat exported files as local device-usage data
and share them deliberately.

## Current status

These are design commitments for planned functionality. During Phase 3, the
Tauri desktop shell continues to display only bundled simulated fixtures. It
does not create a database, collect battery measurements, install a timer,
export data, start a network service, or send data off the device.
