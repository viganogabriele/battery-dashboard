# Local data model

## Database location and ownership

Battery Dashboard stores history only in the current user's XDG data directory:

```text
${XDG_DATA_HOME:-~/.local/share}/battery-dashboard/battery.sqlite3
```

The database is created by an explicit recorder run. Enabling background
recording is an explicit user action; the 60-second systemd user timer is
disabled by default. The UI does not write periodic samples itself.

## Schema version 1

The database uses SQLite `PRAGMA user_version` migrations. Version 1 contains
the append-only `battery_samples` table. Each row represents one physical
battery at one observed instant and includes:

- `battery_id`, an opaque local identifier such as `BAT0`;
- `recorded_at_utc`, an RFC 3339 UTC timestamp;
- `boot_id` and `boot_seconds`, which let later phases recognise reboot and
  suspend boundaries without interpolating them;
- normalized state: `charging`, `discharging`, `full`, `idle`, or `unknown`;
- optional percentage, energy, full/design capacity, power, voltage, current,
  temperature, time estimate, and cycle count;
- one provenance value per metric: `upower`, `sysfs`, `derived`, or
  `unavailable`.

An unavailable value is stored as `NULL` with `unavailable` provenance. It is
never stored as zero. The schema validates this pairing, rejects invalid states,
and prevents updates or deletes of raw samples. Future derived sessions and
summaries will be rebuildable from this immutable input.

## Duplicate and gap rules

Two unique keys avoid duplicate writes: battery ID plus UTC timestamp, and
battery ID plus boot ID plus boot-relative seconds. A repeated manual run or an
overlapping timer invocation is therefore idempotent.

The database stores actual samples only. A boot ID change, absent sample, timer
downtime, suspend, shutdown, or missing battery is a gap. No rows are invented
to fill it.

## Recent-history reads

The desktop reads a bounded UTC range for the selected physical battery or the
aggregate view. Reads never create the database: before the first recorder run
the dashboard reports that no stored history exists. The query keeps both ends
of a detected gap and downsamples only ordinary points, so a compact chart
cannot silently join two unrelated measurements.

The current live dashboard value may be appended in memory as a `transient`
point. It is visibly distinct from persisted data and is not a recorder write.
If the recorder is disabled, sparse, stale, or unavailable, the UI says so and
does not infer charge, discharge, energy change, or a missing timeline.

## SQLite operation

Each connection enables WAL mode, foreign keys, and a five-second busy timeout.
WAL allows the desktop to read while the short-lived recorder writes, but all
processes must use the same local filesystem. An integrity check is part of the
storage test suite. The application does not depend on the `sqlite3` command
line utility.

## Development smoke test

To run the recorder once without touching normal local history, build it and
give it a temporary XDG data home:

```sh
cargo build --manifest-path src-tauri/Cargo.toml --bin battery-dashboard-recorder
XDG_DATA_HOME="$(mktemp -d)" target/debug/battery-dashboard-recorder
```

The command creates rows only for batteries that are currently discoverable.
It does not enable the timer. The temporary directory can be removed after the
test.
