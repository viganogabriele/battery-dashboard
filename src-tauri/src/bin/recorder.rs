//! Standalone one-shot entry point started by the optional systemd user timer.

#![forbid(unsafe_code)]

fn main() {
    if let Err(error) =
        tauri::async_runtime::block_on(battery_dashboard_desktop::recorder::record_once())
    {
        eprintln!("Battery Dashboard recorder failed: {error}");
        std::process::exit(1);
    }
}
