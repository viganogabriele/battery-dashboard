# Battery Dashboard Agent Guide

This file applies to the entire repository. It is the operational handoff for
coding agents. Read it together with `DEVELOPMENT_PLAN.md`; the plan is the
authoritative product scope and current-status document.

## Start here

Before changing code:

1. Read `README.md`, sections 1–7 and 23–30 of `DEVELOPMENT_PLAN.md`, and the
   documentation relevant to the task.
2. Run `git status --short` and preserve all existing user changes.
3. Inspect the actual implementation and tests. Do not infer completion from a
   phase label or README claim.
4. For a bug, reproduce or gather direct evidence before editing. For a UI
   problem, inspect the affected viewport and real native state when possible.
5. Keep the user informed during long-running builds or tests.

## Product truth

Battery Dashboard is an Arch-first, Linux-portable, local-only native battery
monitor. It uses Tauri 2, Svelte 5, TypeScript, Vite, Tailwind CSS, Rust,
SQLite, UPower, sysfs, and an opt-in `systemd --user` timer.

Current status:

- phases 1–7 are implemented;
- phase 8 core functionality exists, but version 1 release validation and
  packaging are incomplete;
- local anomaly analysis and explicit `powerprofilesctl` controls exist;
- notifications, per-process impact, and the Omarchy plugin do not exist;
- the application is a working beta, not a finished 1.0 release;
- the next priorities are listed in `DEVELOPMENT_PLAN.md` section 23.2.

Do not describe the project as complete until all version-1 UX, lifecycle,
packaging, real-hardware, and documentation acceptance criteria pass.

## Non-negotiable behavior

- Native production screens use real local data only. Simulation belongs in
  deterministic tests and development fixtures, never as a silent runtime
  fallback.
- Missing or unreliable hardware values remain unavailable. Never invent,
  clamp, interpolate across gaps, or turn missing values into zero.
- Estimates must name their evidence. Distinguish UPower estimates from
  historically derived estimates and withhold them when evidence is weak.
- Preserve suspend, reboot, battery-removal, clock-change, and sampling gaps.
  Time before recording began is unrecorded coverage, not a synthetic gap.
- Support every discovered physical power-supply battery; never assume only
  `BAT0` or `BAT1`.
- Keep all data local. No cloud, accounts, telemetry, analytics, crash upload,
  or remote inference.
- Do not add Electron, a permanent HTTP server, a tray icon, or a top-bar icon
  to the core app.
- Do not use `sudo`, `pkexec`, system-wide services, or automatic privilege
  changes. Recorder management is per-user and opt-in.
- Disabling recording stops future samples and preserves SQLite history.
  Deleting history must remain a separate explicit operation.
- Omarchy integration is optional and independent. Never edit
  `/usr/share/omarchy` from the core app.

## Architecture boundaries

- `src/`: Svelte UI, frontend domain types, services, charts, and component
  tests.
- `src-tauri/src/battery/`: UPower/sysfs discovery, validation, normalization,
  and field provenance.
- `crates/battery-core/`: shared platform-neutral Rust domain rules.
- `src-tauri/src/storage/`: migrations, immutable raw samples, history,
  aggregation, sessions, and gap semantics.
- `src-tauri/src/bin/recorder.rs`: short-lived one-shot recorder. The desktop
  app must not become a second periodic writer.
- `src-tauri/src/scheduler.rs` and `src-tauri/src/recorder_install.rs`: safe
  per-user systemd lifecycle.
- `src-tauri/src/main.rs`: typed Tauri command boundary. Keep filesystem and
  process authority in Rust, not the webview.
- `systemd/`: source unit templates. Runtime units live below the user's XDG
  configuration directory.
- `packaging/`: desktop integration artifacts.
- `integrations/`: optional integrations only; the core must not depend on
  them.

The default database is
`${XDG_DATA_HOME:-$HOME/.local/share}/battery-dashboard/battery.sqlite3`.
Tests must use temporary paths and must never read, overwrite, migrate, or purge
the user's real database.

## UX rules

Hyprland tiled windows are a primary acceptance environment even though the app
must remain compositor-independent.

- Verify at 640×520, 800×600, 960×640, 1280×820, and maximized sizes.
- No text overlap, accidental shrinking, clipped controls, horizontal page
  scrolling, or huge empty/unavailable cards.
- Keep current percentage/state, watts, runtime/time-to-full evidence, and the
  recent chart above secondary details.
- Do not repeat navigation destinations as dashboard filler.
- Hide unavailable secondary metrics when their absence is not actionable;
  explain important unavailable values once and clearly.
- Charts must use readable domains, show coverage, render real gaps, and provide
  textual or table alternatives.
- Avoid layout-changing periodic refresh. The current UI uses explicit manual
  refresh as a stability measure. Any automatic refresh must first be measured,
  pause when hidden, avoid re-querying unrelated SQLite views, and not cause
  visible WebKitGTK hitches.
- Practical history answers must state sample count, coverage, range, and
  confidence. Do not promise duration from every percentage until enough
  complete real sessions exist.

## Development commands

Install dependencies once:

```sh
pnpm install
```

Frontend quality gates:

```sh
pnpm format:check
pnpm lint
pnpm check
pnpm test
pnpm build
```

Rust quality gates:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Native development and release:

```sh
pnpm tauri dev
pnpm tauri build
```

If port 5173 is occupied, identify and stop only the stale process owned by
this checkout. Do not kill unrelated development servers.

Run focused tests while iterating, then run every applicable quality gate
before claiming completion. A release-affecting change also requires a Tauri
release build and a native smoke test.

## Implementation workflow

- Build from the simplest correct behavior toward richer analysis.
- Keep Rust domain/query logic deterministic and test it with fixtures. Keep
  Svelte components focused on presentation and interaction.
- Prefer typed DTOs and explicit availability/reason fields over sentinel
  strings or magic numbers.
- Treat raw samples as immutable. Derived sessions and summaries may be rebuilt
  idempotently.
- Preserve metric units and provenance at every boundary. Never mix charge in
  Ah with energy in Wh without a valid explicit conversion.
- Keep the recorder executable compatible across app updates and stage it to a
  stable per-user path when recording is enabled.
- Update `DEVELOPMENT_PLAN.md`, README, and relevant docs when behavior, status,
  packaging, privacy, or limitations change.
- Add or update tests for every meaningful correction, including missing-data
  and multiple-battery paths.

## Git and local installation safety

- The worktree may contain user work. Never discard or rewrite unrelated
  changes.
- Do not use destructive Git commands. Do not amend, rebase, force-push, or push
  unless the current user request authorizes it.
- The installed executable under `~/.local/bin` is not updated by changing
  source files. Rebuild and reinstall only when the task asks for a usable local
  build, and report exactly what was installed.
- Never remove the SQLite database during normal install, update, disable, or
  uninstall testing.

## Definition of done

A change is done only when:

- the requested behavior is implemented without weakening data truth or
  privacy constraints;
- relevant frontend and Rust tests pass;
- formatting, linting, and type checks pass;
- native/release behavior is tested in proportion to the risk;
- responsive and unavailable/error states are checked for UI changes;
- documentation and phase status match reality;
- the final handoff lists changed files, verification performed, remaining
  limitations, and whether the installed local app was updated.
