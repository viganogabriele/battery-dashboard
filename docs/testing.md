# Testing strategy

## Principle

The project will test deterministic simulated data and real hardware. Hardware
checks are valuable but never replace fixtures: a developer laptop cannot cover
the range of Linux battery drivers and incomplete firmware data.

## Quality commands

The Phase 1 scaffold provides the following commands. Every applicable phase
must run them:

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

Phase-specific commands will be added without removing these baseline gates.

## Fixture coverage

The Rust provider layer will use injected sysfs roots and mocked UPower clients.
The frontend will use typed fixtures. Together they will cover at least:

- energy-based and charge-based batteries;
- single and multiple batteries, including non-`BAT0` names;
- charging, discharging, full, idle, mixed, and unknown states;
- missing temperature, power, cycle count, UPower, or a battery itself;
- valid derived power from current and voltage;
- invalid, sentinel, overflow, and out-of-range data;
- suspension, removal/reinsertion, reboot, clock correction, and DST gaps;
- recorder-disabled, empty-database, and temporarily locked-database states.

## Phase-specific verification

Early frontend work will test types, formatting, aggregation, components,
responsive behavior, and accessibility states with simulated data. The Tauri
phase will verify the static bundle, one-window behavior, no tray icon, and
clean process exit.

Phase 3 verifies the static Tauri build, its one-window/no-tray configuration,
and a short launch of the compiled desktop executable. The existing fixture
tests remain the source of truth for the simulated dashboard; no real-hardware
assertion is introduced by the desktop shell.

Phase 4 adds deterministic sysfs and UPower mapping tests, including multiple
batteries, non-battery supplies, missing fields, malformed values, sentinels,
and source precedence. It also uses a live Arch hardware smoke test through the
native Tauri command. The smoke test confirms only that the current machine can
be read; it does not turn its firmware fields into a portability guarantee.

Phase 5 adds migration, duplicate/rollback, integrity, one-shot recorder,
systemd user-unit template, and explicit staging checks. The recorder is also
smoke-tested against real UPower/sysfs data using a temporary XDG data home, so
test samples do not alter normal user history. Phase 6 adds bounded-range,
downsampling, per-battery gap, empty-history, transient-live-point, and chart
state tests. Its browser check verifies the simulated preview remains clearly
separate from the native recorder-backed chart. Later phases will add session
reconstruction, timezone/DST aggregation, health calculation, CSV escaping,
JSON round trips, and installer lifecycle tests.

## Real hardware checks

When real providers are introduced, tests will verify the detected devices,
data sources, available fields, missing fields, and clean failure behavior on
an available Arch system. Findings will be recorded as a phase completion
report without treating hardware-specific observations as universal behavior.

## Completion criteria

Every development phase reports implemented work, files changed, commands and
results, real-hardware observations where relevant, and known limitations. The
full required matrix is maintained in the
[development plan](../DEVELOPMENT_PLAN.md#26-required-simulation-matrix).
