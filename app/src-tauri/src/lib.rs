//! Home Advisor desktop shell.
//!
//! The shell owns no domain logic and opens no port: every Tauri command is
//! a thin adapter over the `ha-*` crates, reached through Tauri IPC only
//! (spec, milestone 2: "Tauri IPC replaces any HTTP listener"). The
//! frontend renders state; the Rust core keeps sole ownership of the store,
//! the gate, and the sidecar.
//!
//! - [`commands`] — the IPC surface: the read commands, their views, and
//!   the single-sourced registration the surface audit pins.
//! - [`state`] — the managed `AppState`: the encrypted store handle and the
//!   loopback scan client.
//! - [`audit`] — the IPC surface audit: no command accepts raw SQL, file
//!   paths, or network targets from the webview.

use tauri::Manager as _;

// The surface audit is test infrastructure: it runs in CI, never in the
// shipped binary.
#[cfg(test)]
mod audit;
mod commands;
mod state;

#[cfg(test)]
mod command_tests;

/// Runs the desktop app.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::expect_used)] // builder failure at startup is unrecoverable; the template aborts with a diagnostic
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(commands::invoke_handler())
        .setup(|app| {
            // Fail-closed startup: `open_state` needs the database key and
            // aborts with a diagnostic when it is absent — the app never
            // runs against a plaintext or empty store. First-run key
            // creation belongs to onboarding.
            app.manage(state::open_state(app.handle())?);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run the Home Advisor application");
}
