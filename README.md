# Battery Dashboard

Battery Dashboard is a native Linux desktop application for observing
laptop battery state and battery history. Its purpose is to make available
hardware data understandable without cloud services, telemetry, accounts, a
permanent web server, or elevated privileges.

The project is **Arch-first and Linux-portable by design**. Version 1 will be
tested and supported on Arch Linux and its derivatives with a systemd user
session. The core data, storage, and UI layers deliberately do not depend on
Omarchy, Hyprland, GNOME, KDE, or a particular status bar. Other Linux
distributions may be compatible when their system libraries and battery
interfaces are supported, but they are not yet official targets.

## Current status

The usable desktop application is complete for its local-only scope. It has one
normal window, no tray or top-bar icon, and no production HTTP server. It reads
physical laptop batteries from UPower and Linux sysfs, falls back per metric,
and retains the source of every value.

The app includes a live dashboard; two-, six-, twelve-, and twenty-four-hour
charge charts; charge/discharge sessions; daily, weekly, and monthly history;
capacity, hardware-cycle, and conservative degradation reports; CSV/JSON
export; evidence-based anomaly checks; and Linux power-profile controls when
`powerprofilesctl` is available. Persistent history is opt-in via a short-lived
recorder launched every 60 seconds by a `systemd --user` timer. Gaps caused by
suspend, reboot, missing samples, or different batteries are retained instead
of being interpolated.

The product will be built incrementally: simulated UI first, then Tauri,
real battery data, persistent recording, history, health, and export. The
authoritative scope and phase order are in [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md).

## Available capabilities

The following are available where local hardware exposes sufficient data:

- live percentage, state, power, voltage, current, and battery temperature;
- remaining-time estimates only when supported or well evidenced;
- charts, charge/discharge sessions, and calendar history;
- health, maximum/design capacity, hardware cycle count, and conservative
  degradation trends;
- CSV and JSON export;
- single-battery and multiple-battery views;
- optional, user-controlled background sampling every 60 seconds;
- local `powerprofilesctl` profile selection and conservative recorded-history
  anomaly reporting.

Unavailable or unreliable values will be shown as such, never silently
invented or treated as zero.

## Stack

- Tauri 2 and Rust for the native desktop application and typed IPC;
- Svelte 5, TypeScript, Vite, and Tailwind CSS for the interface;
- UPower and Linux `/sys/class/power_supply` as complementary data sources;
- SQLite for local history;
- a `systemd --user` timer for optional one-shot background recording.

The production app will bundle static frontend assets. Vite is only a
development server; the product will not run an HTTP server after installation.
The desktop app will not create a tray or top-bar icon.

## Repository layout

```text
src/                  Svelte interface and frontend tests
crates/battery-core/  Shared platform-neutral Rust domain types
src-tauri/            Tauri application (introduced in Phase 3)
systemd/              User unit templates (introduced in Phase 5)
tests/                Deterministic fixtures (expanded with each phase)
docs/                 Architecture, privacy, testing, and hardware docs
integrations/         Optional integrations; Omarchy is post-v1 only
```

Some directories are introduced as their corresponding implementation phases
begin; see the [planned layout](DEVELOPMENT_PLAN.md#8-planned-repository-layout).

## Prerequisites and development workflow

The project requires Rust 1.85 or newer, Node.js 22 or newer, and pnpm 11.
For Arch Linux development, Tauri's official Linux prerequisites currently
list `webkit2gtk-4.1`, `base-devel`, `curl`, `wget`, `file`, `openssl`,
`appmenu-gtk-module`, `libappindicator-gtk3`, `librsvg`, and `xdotool`.
Install system packages with your normal package-management process; this is a
development-machine requirement, not a request for the application to use
`sudo`, `pkexec`, or any privileged helper. See the
[official Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
for the current package list and non-Arch distribution instructions.

The current quality workflow is:

```sh
pnpm install
pnpm check
pnpm test
pnpm build
cargo test --workspace
```

Start the native development window with `pnpm tauri dev` and build the desktop
bundle with `pnpm tauri build`. These commands are not a web deployment
workflow: Vite serves frontend assets only during development. The installed
application loads its packaged static assets directly and does not start an HTTP
server.

### Local user install and removal

`pnpm tauri build` builds the release executable. To make a build from this
checkout visible in the application menu for the current user, install its
binary, desktop entry, and icon under user-owned XDG paths. The exact commands
are documented rather than run by the application:

```sh
install -Dm755 target/release/battery-dashboard-desktop ~/.local/bin/battery-dashboard
install -Dm755 target/release/battery-dashboard-recorder \
  ~/.local/bin/battery-dashboard-recorder
install -Dm644 src-tauri/icons/icon.png \
  ~/.local/share/icons/hicolor/512x512/apps/battery-dashboard.png
install -Dm644 packaging/com.gabrielevigano.batterydashboard.desktop \
  ~/.local/share/applications/com.gabrielevigano.batterydashboard.desktop
update-desktop-database ~/.local/share/applications
```

Afterwards, launch **Battery Dashboard** from the application menu or run
`battery-dashboard`. `pnpm tauri dev` also builds the separate recorder binary,
but does **not** enable recording. In Settings, enabling it explicitly stages a
copy below the user's XDG data directory and creates user units below the XDG
config directory; it then uses `systemctl --user` to enable the timer.
Disabling stops future samples and preserves history.

To remove the app files, remove the four files installed above. To disable the
recorder first, use Settings or run `systemctl --user disable --now
battery-dashboard-recorder.timer`. The database is deliberately retained until
the user explicitly removes
`${XDG_DATA_HOME:-~/.local/share}/battery-dashboard/battery.sqlite3`; do not
delete it unless intentionally purging local history.

## Privacy and background recording

All measurements remain on the local machine. Background recording is disabled
by default and must be explicitly enabled in the desktop Settings screen. It is
managed with `systemctl --user`, not `sudo` or `pkexec`; disabling it preserves
history unless the user explicitly purges it. The recorder is a short-lived
process every 60 seconds, not a daemon or web server.

Read [the privacy policy](docs/privacy.md) and [the architecture overview](docs/architecture.md)
for the intended data flow and boundaries.

## Omarchy

Omarchy is not required to use Battery Dashboard. A compact Omarchy bar plugin
is an **optional post-version-1 integration**, separately installed and
removable. The core app will not modify `/usr/share/omarchy` or create any
bar item itself.

## Documentation

- [Product and development plan](DEVELOPMENT_PLAN.md)
- [Architecture](docs/architecture.md)
- [Local data model](docs/data-model.md)
- [Hardware support and limitations](docs/hardware-support.md)
- [Privacy](docs/privacy.md)
- [Testing strategy](docs/testing.md)

## License

No license has been selected yet.
