# Battery Dashboard

Battery Dashboard is a planned native Linux desktop application for observing
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

Phases 1–3 are complete. The repository includes the Tauri 2 desktop shell,
the Svelte/Vite/Tailwind scaffold, a small platform-neutral Rust domain crate, a
responsive dashboard, metric cards, SVG charts, typed fixture scenarios, and
navigation placeholders for later screens. The scenario selector exercises
single and multiple batteries, charging, incomplete telemetry, stale suspend
data, and no-battery states without accessing hardware.

The Tauri desktop shell has one normal window, no tray or top-bar icon, and no
production HTTP server. There is still no live battery reader, database,
recorder, export, or Omarchy plugin. The simulated data is deliberately visible
in the UI and does not read or store battery data.

The product will be built incrementally: simulated UI first, then Tauri,
real battery data, persistent recording, history, health, and export. The
authoritative scope and phase order are in [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md).

## Planned capabilities

Version 1 is planned to provide, where hardware exposes sufficient data:

- live percentage, state, power, voltage, current, and battery temperature;
- remaining-time estimates only when supported or well evidenced;
- charts, charge/discharge sessions, and calendar history;
- health, maximum/design capacity, hardware cycle count, and conservative
  degradation trends;
- CSV and JSON export;
- single-battery and multiple-battery views;
- optional, user-controlled background sampling every 60 seconds.

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

### Temporary Phase 3 install and removal

Packaging and a supported installer are not available yet. Until they are,
use the development command above rather than treating build output as a
system-wide installation. Removing the development checkout or generated build
artifacts does not remove any user battery history, because this phase does not
create a database or recorder. A per-user installer, uninstaller, desktop
entry, and explicit history purge workflow are planned for the release phase.

## Privacy and background recording

All measurements are designed to remain on the local machine. The Phase 3
desktop shell does not add network access, cloud communication, telemetry, or
background recording. Later recording will be disabled by default and
explicitly enabled by the user. It will be managed with `systemctl --user`,
not `sudo` or `pkexec`; disabling it will preserve history unless the user
explicitly purges it.

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
- [Hardware support and limitations](docs/hardware-support.md)
- [Privacy](docs/privacy.md)
- [Testing strategy](docs/testing.md)

## License

No license has been selected yet.
