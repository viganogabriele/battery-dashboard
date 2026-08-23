CREATE TABLE IF NOT EXISTS battery_samples (
    id INTEGER PRIMARY KEY,
    battery_id TEXT NOT NULL CHECK (length(trim(battery_id)) > 0),
    recorded_at_utc TEXT NOT NULL,
    boot_id TEXT NOT NULL CHECK (length(trim(boot_id)) > 0),
    boot_seconds REAL NOT NULL CHECK (boot_seconds >= 0),
    state TEXT NOT NULL CHECK (state IN ('charging', 'discharging', 'full', 'idle', 'unknown')),
    percentage REAL,
    percentage_source TEXT NOT NULL CHECK (percentage_source IN ('upower', 'sysfs', 'derived', 'unavailable')),
    energy_now_wh REAL,
    energy_now_wh_source TEXT NOT NULL CHECK (energy_now_wh_source IN ('upower', 'sysfs', 'derived', 'unavailable')),
    energy_full_wh REAL,
    energy_full_wh_source TEXT NOT NULL CHECK (energy_full_wh_source IN ('upower', 'sysfs', 'derived', 'unavailable')),
    energy_design_wh REAL,
    energy_design_wh_source TEXT NOT NULL CHECK (energy_design_wh_source IN ('upower', 'sysfs', 'derived', 'unavailable')),
    power_watts REAL,
    power_watts_source TEXT NOT NULL CHECK (power_watts_source IN ('upower', 'sysfs', 'derived', 'unavailable')),
    voltage_volts REAL,
    voltage_volts_source TEXT NOT NULL CHECK (voltage_volts_source IN ('upower', 'sysfs', 'derived', 'unavailable')),
    current_amps REAL,
    current_amps_source TEXT NOT NULL CHECK (current_amps_source IN ('upower', 'sysfs', 'derived', 'unavailable')),
    temperature_celsius REAL,
    temperature_celsius_source TEXT NOT NULL CHECK (temperature_celsius_source IN ('upower', 'sysfs', 'derived', 'unavailable')),
    time_remaining_minutes REAL,
    time_remaining_minutes_source TEXT NOT NULL CHECK (time_remaining_minutes_source IN ('upower', 'sysfs', 'derived', 'unavailable')),
    cycle_count REAL,
    cycle_count_source TEXT NOT NULL CHECK (cycle_count_source IN ('upower', 'sysfs', 'derived', 'unavailable')),
    CHECK ((percentage IS NULL) = (percentage_source = 'unavailable')),
    CHECK ((energy_now_wh IS NULL) = (energy_now_wh_source = 'unavailable')),
    CHECK ((energy_full_wh IS NULL) = (energy_full_wh_source = 'unavailable')),
    CHECK ((energy_design_wh IS NULL) = (energy_design_wh_source = 'unavailable')),
    CHECK ((power_watts IS NULL) = (power_watts_source = 'unavailable')),
    CHECK ((voltage_volts IS NULL) = (voltage_volts_source = 'unavailable')),
    CHECK ((current_amps IS NULL) = (current_amps_source = 'unavailable')),
    CHECK ((temperature_celsius IS NULL) = (temperature_celsius_source = 'unavailable')),
    CHECK ((time_remaining_minutes IS NULL) = (time_remaining_minutes_source = 'unavailable')),
    CHECK ((cycle_count IS NULL) = (cycle_count_source = 'unavailable')),
    UNIQUE (battery_id, recorded_at_utc),
    UNIQUE (battery_id, boot_id, boot_seconds)
);

CREATE INDEX IF NOT EXISTS battery_samples_recorded_at_idx
    ON battery_samples (recorded_at_utc DESC);

CREATE INDEX IF NOT EXISTS battery_samples_battery_time_idx
    ON battery_samples (battery_id, recorded_at_utc DESC);

CREATE TRIGGER IF NOT EXISTS battery_samples_no_update
BEFORE UPDATE ON battery_samples
BEGIN
    SELECT RAISE(ABORT, 'battery samples are immutable');
END;

CREATE TRIGGER IF NOT EXISTS battery_samples_no_delete
BEFORE DELETE ON battery_samples
BEGIN
    SELECT RAISE(ABORT, 'battery samples are immutable');
END;
