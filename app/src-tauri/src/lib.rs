//! Home Advisor desktop shell.
//!
//! The shell owns no domain logic and opens no port: every Tauri command is a
//! thin adapter over the `ha-*` crates, reached through Tauri IPC only (spec,
//! milestone 2: "Tauri IPC replaces any HTTP listener"). The frontend renders
//! state; the Rust core keeps sole ownership of the store, the gate, and the
//! sidecar.

/// Placeholder command demonstrating the IPC surface; replaced by store and
/// gate adapters as the milestone-2 slices land.
///
/// Private visibility is deliberate: `generate_handler!` resolves it in this
/// module, and a `pub` command makes the macro emit a same-module macro
/// re-import that collides in the macro namespace (E0255).
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {name}. Home Advisor is running locally — nothing leaves this machine.")
}

/// Runs the desktop app.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::expect_used)] // builder failure at startup is unrecoverable; the template aborts with a diagnostic
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("failed to run the Home Advisor application");
}
