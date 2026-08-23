-- Derived cache only. `battery_samples` remains immutable source telemetry.
CREATE TABLE IF NOT EXISTS battery_sessions (
    id INTEGER PRIMARY KEY,
    battery_id TEXT NOT NULL CHECK (length(trim(battery_id)) > 0),
    kind TEXT NOT NULL CHECK (kind IN ('charging', 'discharging', 'full', 'unknown')),
    started_at_utc TEXT NOT NULL,
    ended_at_utc TEXT NOT NULL,
    sample_count INTEGER NOT NULL CHECK (sample_count > 0),
    observed_duration_seconds REAL,
    start_percentage REAL,
    end_percentage REAL,
    start_energy_wh REAL,
    end_energy_wh REAL,
    average_power_watts REAL,
    complete INTEGER NOT NULL CHECK (complete IN (0, 1)),
    interrupt_reason TEXT NOT NULL CHECK (interrupt_reason IN ('state_changed', 'boot_changed', 'sample_gap', 'data_ended')),
    CHECK (ended_at_utc >= started_at_utc)
);

CREATE INDEX IF NOT EXISTS battery_sessions_battery_time_idx
    ON battery_sessions (battery_id, started_at_utc DESC);

CREATE INDEX IF NOT EXISTS battery_sessions_time_idx
    ON battery_sessions (started_at_utc DESC);
