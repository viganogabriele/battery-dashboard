//! Native desktop entry point for Battery Dashboard.

#![forbid(unsafe_code)]

use battery_dashboard_desktop::{
    battery, recorder_install,
    scheduler::{SchedulerStatus, SystemdUserScheduler},
    storage,
};
use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Returns current battery readings without persisting or altering system state.
#[tauri::command]
async fn get_battery_dashboard() -> battery::BatteryDashboardResponse {
    battery::read_dashboard().await
}

/// Returns the opt-in background recorder state without creating a database.
#[tauri::command]
fn get_recorder_status() -> RecorderStatusResponse {
    recorder_status(None)
}

/// Explicitly enables or disables background recording for the current user.
///
/// Enabling stages the recorder and its systemd user units under XDG paths,
/// then asks the existing user manager to enable the timer. Disabling preserves
/// both the units and history while stopping future samples.
#[tauri::command]
fn set_recorder_enabled(enabled: bool) -> RecorderStatusResponse {
    let scheduler = SystemdUserScheduler::for_current_user();
    if enabled {
        if let SchedulerStatus::Unavailable { reason } = scheduler.status() {
            return recorder_status(Some(format!(
                "background recording is unsupported on this system: {reason}"
            )));
        }
    }

    let result = if enabled {
        recorder_install::stage_built_recorder().and_then(|_| {
            scheduler.enable().map_err(|error| {
                recorder_install::RecorderInstallError::Io(std::io::Error::other(error))
            })
        })
    } else {
        scheduler.disable().map_err(|error| {
            recorder_install::RecorderInstallError::Io(std::io::Error::other(error))
        })
    };

    match result {
        Ok(()) => recorder_status(None),
        Err(error) => recorder_status(Some(error.to_string())),
    }
}

/// The stable frontend contract for recorder control and diagnostics.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecorderStatusResponse {
    schema_version: u8,
    supported: bool,
    enabled: bool,
    transition: &'static str,
    health: &'static str,
    last_recorded_at: Option<String>,
    error: Option<String>,
}

fn recorder_status(request_error: Option<String>) -> RecorderStatusResponse {
    let scheduler_status = SystemdUserScheduler::for_current_user().status();
    let (supported, enabled, scheduler_error) = match scheduler_status {
        SchedulerStatus::Enabled => (true, true, None),
        SchedulerStatus::Disabled => (true, false, None),
        SchedulerStatus::Unavailable { reason } => (false, false, Some(reason)),
    };

    if !supported {
        return RecorderStatusResponse {
            schema_version: 1,
            supported: false,
            enabled: false,
            transition: "idle",
            health: "unknown",
            last_recorded_at: None,
            error: request_error.or(scheduler_error),
        };
    }

    match storage::last_recorded_at_if_exists() {
        Ok(last_recorded_at) => RecorderStatusResponse {
            schema_version: 1,
            supported: true,
            enabled,
            transition: "idle",
            health: if enabled && last_recorded_at.is_some() {
                "healthy"
            } else {
                "unknown"
            },
            last_recorded_at: last_recorded_at.and_then(format_timestamp),
            error: request_error,
        },
        Err(error) => RecorderStatusResponse {
            schema_version: 1,
            supported: true,
            enabled,
            transition: "idle",
            health: "error",
            last_recorded_at: None,
            error: request_error.or_else(|| Some(error.to_string())),
        },
    }
}

fn format_timestamp(timestamp: OffsetDateTime) -> Option<String> {
    timestamp
        .to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .ok()
}

/// Creates the desktop application builder.
fn app_builder() -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default().invoke_handler(tauri::generate_handler![
        get_battery_dashboard,
        get_recorder_status,
        set_recorder_enabled
    ])
}

fn main() {
    app_builder()
        .run(tauri::generate_context!())
        .expect("failed to run Battery Dashboard");
}

#[cfg(test)]
mod tests {
    use super::app_builder;

    #[test]
    fn desktop_builder_can_be_created_without_hardware_access() {
        let _builder = app_builder();
    }
}
