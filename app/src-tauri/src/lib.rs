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
//! - [`state`] — the managed `AppState`: the encrypted store handle, the
//!   loopback scan client, and the sidecar supervisor.
//! - [`keystore`] — where the database key comes from: the Keychain on
//!   macOS, the env override everywhere, fail-closed diagnostics always.
//! - [`supervisor`] — the sidecar lifecycle machine: spawn, health-check,
//!   restart; `privacy_status` and the egress pre-flight both read it.
//! - [`egress`] — the app's one gated egress path: supervision pre-flight,
//!   then the `ha-privacy` pipeline; fail-closed by construction.
//! - [`audit`] — the IPC surface audit: no command accepts raw SQL, file
//!   paths, or network targets from the webview.

use tauri::Manager as _;

// The surface audit is test infrastructure: it runs in CI, never in the
// shipped binary.
#[cfg(test)]
mod audit;
mod commands;
mod egress;
mod keystore;
mod onboarding;
mod packs;
mod state;
mod supervisor;

#[cfg(test)]
mod command_tests;

#[cfg(test)]
mod keystore_tests;

#[cfg(test)]
mod onboarding_tests;

/// Runs the desktop app.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::expect_used)] // builder failure at startup is unrecoverable; the template aborts with a diagnostic
pub fn run() {
    let app = tauri::Builder::default()
        // Local notifications (the advice-pack ladder): no network path —
        // the desktop plugin posts to the OS notification center.
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(commands::invoke_handler())
        .build(tauri::generate_context!())
        .expect("failed to build the Home Advisor application");

    // State bootstrap *before* the event loop, deliberately: on macOS the
    // Tauri setup hook runs inside `did_finish_launching`, where a returned
    // error unwinds across the Objective-C boundary and aborts the process
    // before any window exists — the v0.1.0 first-run SIGABRT. Here a
    // fail-closed startup prints its cause and exits nonzero, cleanly.
    match state::open_state(app.handle()) {
        Ok(app_state) => {
            app.manage(app_state);
        }
        Err(error) => {
            eprintln!("Home Advisor could not start.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }

    app.run(|app_handle, event| {
        // Kill the supervised sidecar on exit: a closed window must not
        // orphan the child process the app spawned. This is process
        // cleanup, not fail-closure — the sidecar is our own child.
        if let tauri::RunEvent::Exit = event {
            if let Some(state) = app_handle.try_state::<state::AppState>() {
                state.shutdown();
            }
        }
    });
}
