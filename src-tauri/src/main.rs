//! Native desktop entry point for Battery Dashboard.

#![forbid(unsafe_code)]

mod battery;

/// Returns current battery readings without persisting or altering system state.
#[tauri::command]
async fn get_battery_dashboard() -> battery::BatteryDashboardResponse {
    battery::read_dashboard().await
}

/// Creates the desktop application builder.
fn app_builder() -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default().invoke_handler(tauri::generate_handler![get_battery_dashboard])
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
