# Architecture

## Intent

Battery Dashboard is a native desktop application, not a web service. The UI
is packaged inside a Tauri window and talks to typed Rust commands. No
production localhost server is required or planned.

```text
UPower system D-Bus ----+
                         +--> Rust battery providers --> Tauri IPC --> Svelte UI
/sys/class/power_supply -+             |
                                       +--> SQLite (reads)

systemd --user timer --> one-shot Rust recorder --> SQLite (write) --> exit
```

Phase 3 introduces the desktop shell around the existing simulated UI. It is
configured as one normal application window: closing it exits the application,
and it has no tray or top-bar icon. The shell does not start a production
localhost server; Vite remains development-only and packaged builds load static
assets through Tauri.

The desktop window will be responsible for live display once live providers
are implemented. The recorder will be a separate one-shot executable so
history can continue to be collected while the window is closed. It is never a
custom daemon.

## Portable boundaries

The first supported scheduler is `systemd --user`, because it is standard on
the initial Arch Linux target. Scheduling is kept behind a backend boundary so
other Linux schedulers can be evaluated later without rewriting collection or
storage. On a system without a supported scheduler, the app must report that
background recording is unavailable while still allowing a live dashboard.

Battery access is also separated into providers:

- `UPowerProvider` reads standardized values from the system D-Bus when
  available.
- `SysfsProvider` reads Linux power supplies under `/sys/class/power_supply`
  and is the fallback for missing UPower fields.
- `CompositeProvider` validates and combines individual fields while retaining
  source provenance.

Providers discover batteries by device type rather than assuming `BAT0`. Tests
will inject fixture roots and mock clients instead of relying on the developer
machine.

## Data rules

Internal units are watts, watt-hours, volts, amperes, degrees Celsius, and a
0–100 percentage. Missing values are represented as unavailable, not as zero.
Charge and energy values are never mixed without a valid, explicit conversion.

Each metric will have a value, source (`upower`, `sysfs`, `derived`, or
`unavailable`), and update time. Derived values are allowed only when their
inputs are compatible and physically meaningful.

Physical batteries remain distinct. An aggregate view will sum compatible
energy and power values, calculate percentage as capacity-weighted, and retain
per-device voltage, current, and temperature rather than manufacturing a
misleading single aggregate value.

## Persistence and lifecycle

SQLite will use versioned migrations, foreign keys, WAL mode, a busy timeout,
and short transactions. It will store raw samples, battery metadata, derived
sessions, and summaries under the XDG data directory:

```text
${XDG_DATA_HOME:-~/.local/share}/battery-dashboard/battery.sqlite3
```

Samples will contain UTC time, Linux boot ID, and boot-relative time where
available. Suspends, shutdowns, reboots, removed batteries, and large
collection gaps create real boundaries; they are not reconstructed by
interpolation.

Background recording will be opt-in. Its eventual per-user paths are:

```text
~/.local/libexec/battery-dashboard/recorder
~/.config/systemd/user/battery-dashboard-recorder.service
~/.config/systemd/user/battery-dashboard-recorder.timer
```

## Security boundaries

Tauri capabilities are being configured restrictively in Phase 3. The webview
must not receive generic shell execution or generic home-directory filesystem
access. Rust commands will own future battery reads, database operations,
scheduler management, and user-initiated exports. The app has one normal window
and no tray icon.

The optional future Omarchy integration is outside the core runtime. It may
consume a stable read-only interface, but must not own the recorder or change
Omarchy packaged files.

For the complete design, including sessions and exports, read the
[development plan](../DEVELOPMENT_PLAN.md).
