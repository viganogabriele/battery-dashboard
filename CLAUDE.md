# Battery Dashboard — Claude Code Handoff

Read `AGENTS.md` first and follow it for all repository work. Then read
`DEVELOPMENT_PLAN.md`, especially sections 1, 4–5, 23, 25, and 28. Those files
are the source of truth; do not rely on an old chat summary.

## Current state

This is a working beta. Real UPower/sysfs data, SQLite recording, charts,
sessions, calendar history, health/degradation, export, local anomalies, and
manual `powerprofilesctl` controls are implemented. Version 1 is not release
complete.

A real-hardware polish session (2026-08-24) fixed several concrete bugs found
by actually running the app and reading the user's own database — see
`AGENTS.md`'s "Current status" for the list (UPower pending-state mapping,
button contrast, dotted chart line, aggregate gap/completeness, Health-view
capacity mismatch) — and added evidence-based observed-usage features
(battery-life-on-a-full-charge, today-vs-yesterday, a historically-derived
live runtime forecast distinct from UPower's own estimate), longer history
ranges (3d/7d/30d), restyled selects, and a general copy-noise cleanup. Do not
assume any of that is still outstanding; verify against the current code
instead of this note before redoing it.

Open work, in order:

1. give Sessions and History a dedicated comprehensibility pass — a real user
   reported "I don't understand anything" about both views; they are
   functional but were never redesigned for clarity (labels, grouping, date
   context beyond what the 2026-08-24 session added);
2. validate recorder/update/data-preservation behavior and real hardware;
3. keep improving useful observed answers: charge/discharge curves, more
   evidence/coverage detail;
4. finish responsive UX at Hyprland tiled sizes without periodic layout hitches
   (spot-checked repeatedly during the 2026-08-24 session with no issues found
   beyond what was fixed, but not a formal sign-off);
5. automate per-user install/update/uninstall/purge and add an Arch packaging
   path;
6. only then add notifications, investigate per-process impact, or build the
   optional Omarchy plugin.

The detailed implementation audit and acceptance criteria are in
`DEVELOPMENT_PLAN.md` sections 23.1–23.3.

## Required product constraints

- Native runtime shows real data or a precise error; never silently substitute
  simulated fixtures.
- Never fabricate estimates or join lines across missing/suspend/reboot data.
- Preserve multiple-battery semantics and metric provenance.
- No sudo, pkexec, cloud, accounts, telemetry, permanent server, tray, or core
  Omarchy dependency.
- Recording is opt-in, runs as a one-shot systemd user timer, and retains
  history when disabled or when the app is upgraded.
- The current manual refresh is intentional because previous broad polling
  caused WebKitGTK/Hyprland hitches. Do not reintroduce polling without measured
  proof that it is scoped and smooth.

## New-session checklist

```sh
git status --short
git log --oneline -8
systemctl --user status battery-dashboard-recorder.timer --no-pager
```

Inspect the relevant code and reproduce the issue before editing. Do not modify
or purge the user's database while testing. Use temporary databases and
fixtures for automated tests.

Run these gates before completion:

```sh
pnpm format:check
pnpm lint
pnpm check
pnpm test
pnpm build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

For release or desktop behavior changes, also run `pnpm tauri build` and perform
a native smoke test. Verify UI changes at 640×520 and 1280×820 at minimum, with
special attention to overflow, shrinking text, duplicated sections, empty-card
noise, chart coverage, and visible refresh hitches.

At the end, report what changed, files touched, tests run, remaining gaps, and
whether a local executable was rebuilt/installed. Do not call the app complete
until the version-1 acceptance criteria in the plan pass.
