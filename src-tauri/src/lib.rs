//! Shared native application services for Battery Dashboard.
//!
//! The desktop window and the one-shot recorder use the same battery provider
//! and storage layers. Keeping those layers in a library prevents the
//! background task from growing a second, inconsistent implementation.

#![forbid(unsafe_code)]

/// Live UPower and sysfs battery providers.
pub mod battery;

/// One-shot live-data recorder shared by the timer binary and desktop app.
pub mod recorder;

/// Explicit per-user staging for the recorder binary and systemd units.
pub mod recorder_install;

/// Per-user systemd timer integration.
pub mod scheduler;

/// Local SQLite persistence for immutable recorder samples.
pub mod storage;
