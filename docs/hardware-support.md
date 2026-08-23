# Hardware support and limitations

## Initial support target

Version 1 will be tested first on Arch Linux and Arch-based distributions with
a systemd user session, a Tauri-compatible WebKitGTK environment, and Linux
power-supply sysfs. UPower is preferred when present, while sysfs remains a
direct source and fallback.

The core application is intentionally independent of Omarchy, Hyprland, GNOME,
KDE, and any particular status bar. Other Linux distributions may work when
their compatible system components are available, but are not supported until
tested.

## Current implementation limit

Phase 5 reads current physical laptop batteries from UPower and
`/sys/class/power_supply`. It excludes UPower's aggregate display device and
non-power-supply peripherals, and it discovers sysfs entries by `type=Battery`.
The UI and recorder are therefore useful with BAT0, BAT1, or differently named
packs. Recorder rows preserve missing values and field provenance; they do not
infer unsupported fields or establish universal hardware compatibility from one
machine.

An unavailable field remains unavailable. In particular, charge values in Ah
are not presented as energy values in Wh unless a later phase has a valid,
explicit conversion rule.

## Expected sources

The planned reader will combine:

- UPower over the system D-Bus, when available;
- `/sys/class/power_supply`, which varies by kernel driver and firmware.

It will discover all devices whose type is battery rather than relying on a
particular name. `BAT0`, `BAT1`, and differently named batteries are expected
to be supported. Each physical battery will remain selectable separately, with
an optional aggregate view.

## Data availability

Hardware and drivers may not expose every metric. Commonly absent or unreliable
fields include battery temperature, current, cycle count, design capacity,
power, and time-to-empty/time-to-full estimates. The app will show an explicit
unavailable, stale, invalid, or still-collecting state instead of fabricating a
value.

Battery health needs both current full capacity and design capacity. Hardware
cycle count will only be shown when reported by the hardware interface; plug
events and partial discharges are not substitutes. A generic ACPI thermal zone
will not be shown as battery temperature unless it can be reliably associated
with the battery.

## Multiple batteries and estimates

Energy and compatible power can be aggregated. Percentage will be weighted by
available full capacity. Voltage, current, and temperature remain per-battery
because a single aggregate value would often be meaningless.

Remaining-time estimates depend on UPower or sufficient uninterrupted local
history. Gaps caused by suspend, reboot, shutdown, battery removal, or missing
sampling are preserved rather than interpolated.

## Background recording

The background recorder needs a reachable systemd user session in version 1.
On non-systemd systems the live dashboard still works, but Settings reports
that persistent recording is unsupported. Alternative scheduler backends are a
post-version-1 portability task.

## How to help validate support

When implementation reaches the hardware phase, useful bug reports will state
distribution and version, kernel version, desktop environment/compositor,
whether UPower is running, and which metrics are unavailable. Do not publish
battery serial numbers or full unredacted diagnostic output without review.
