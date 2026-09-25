//! The application state: the Rust core's handles, held where the webview
//! cannot reach them.
//!
//! `AppState` is the only object the IPC commands touch. It owns the
//! encrypted store (spec, milestone 2: "the Rust core keeps sole ownership
//! of the store, the gate, and the sidecar; the frontend renders state,
//! never data access") and the loopback scan client. Nothing here is
//! serializable or clonable into the webview on purpose.

use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use ha_privacy::LayaSidecar;
use ha_store::Store;
use tauri::Manager as _;

use crate::commands::AppError;

/// How long the privacy-status probe waits for the sidecar to accept a
/// connection before declaring it unavailable. Bounded and short: the probe
/// must never be the slow thing a user notices.
pub(crate) const SIDECAR_PROBE_TIMEOUT: Duration = Duration::from_millis(750);

/// The sidecar endpoint used when the environment does not name one — the
/// same loopback default the CLI documents.
pub const DEFAULT_SIDECAR_URL: &str = "http://127.0.0.1:8000";

pub struct AppState {
    /// The encrypted on-device store. `rusqlite::Connection` is `Send` but
    /// not `Sync`, so the handle sits behind a mutex: one query at a time,
    /// which is all a single-user desktop app needs.
    store: Mutex<Store>,
    /// The Layer 2 scan client. The constructor rejects non-loopback URLs,
    /// so this handle can only ever point at this machine. Sidecar
    /// supervision (spawn, health-check, restart) lands with its own slice;
    /// for now the state carries the client and answers reachability.
    sidecar: LayaSidecar,
}

impl AppState {
    pub fn new(store: Store, sidecar: LayaSidecar) -> Self {
        Self {
            store: Mutex::new(store),
            sidecar,
        }
    }

    /// Lock the store for one query. A poisoned mutex — a query that
    /// panicked mid-flight — is surfaced as an error, never silently
    /// ignored: the gate's fail-closed posture applies to state access too.
    pub(crate) fn lock_store(&self) -> Result<MutexGuard<'_, Store>, AppError> {
        self.store.lock().map_err(|_| AppError::StoreLockPoisoned)
    }

    pub(crate) fn sidecar(&self) -> &LayaSidecar {
        &self.sidecar
    }
}

/// Open the state the running app needs: the store from the configured
/// location, the scan client from the configured loopback endpoint.
///
/// Fail-closed at startup: without a database key the app cannot serve
/// reads, so setup aborts with a diagnostic instead of opening a plaintext
/// or empty store. First-run key creation belongs to onboarding; until that
/// slice lands the key arrives through `HOMEADVISOR_DB_KEY` (the same
/// hand-off the CLI documents).
pub fn open_state(app: &tauri::AppHandle) -> Result<AppState, Box<dyn std::error::Error>> {
    let db_path = match std::env::var("HOMEADVISOR_DB") {
        Ok(path) => std::path::PathBuf::from(path),
        Err(_) => {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            dir.join("homeadvisor.db")
        }
    };

    let key = ha_store::StoreKey::from_env("HOMEADVISOR_DB_KEY")?;
    let store = Store::open(db_path, &key)?;

    let sidecar_url = std::env::var("HOMEADVISOR_SIDECAR_URL")
        .unwrap_or_else(|_| DEFAULT_SIDECAR_URL.to_string());
    let sidecar = LayaSidecar::new(&sidecar_url)?;

    Ok(AppState::new(store, sidecar))
}

/// Is the sidecar reachable right now?
///
/// A bounded TCP connect — a probe, not a scan: it costs no forward pass and
/// answers exactly one question, "is something listening on the loopback
/// endpoint the scan client is configured for". The real scan pipeline
/// (and its failure mapping) is tested in `ha-privacy`.
pub fn probe_sidecar(base_url: &str, timeout: Duration) -> Result<(), String> {
    let host_port = endpoint(base_url)?;
    let addrs: Vec<_> = match host_port.to_socket_addrs() {
        Ok(addrs) => addrs.collect(),
        Err(error) => return Err(format!("could not resolve {host_port}: {error}")),
    };
    for address in addrs {
        if TcpStream::connect_timeout(&address, timeout).is_ok() {
            return Ok(());
        }
    }
    Err(format!(
        "the sidecar at {base_url} is not listening — gated operations stay BLOCKED"
    ))
}

/// Extract `host:port` from a loopback URL like `http://127.0.0.1:8000`.
/// A URL without an explicit port is a configuration error: the probe
/// guesses nothing about which port a sidecar process picked.
fn endpoint(base_url: &str) -> Result<String, String> {
    let without_scheme = base_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .ok_or_else(|| format!("{base_url} is not an http(s) URL"))?;
    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);
    // An endpoint without a port (e.g. a bare `[::1]`) is rejected by
    // `to_socket_addrs`; the probe guesses nothing about which port a
    // sidecar process picked.
    Ok(host_port.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_reaches_a_listening_loopback_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        assert!(probe_sidecar(&base_url, Duration::from_millis(250)).is_ok());
    }

    #[test]
    fn the_probe_reports_a_dead_port() {
        // Take a port, confirm it, then drop the listener: refused for sure.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let base_url = format!("http://127.0.0.1:{port}");
        let detail = probe_sidecar(&base_url, Duration::from_millis(250)).unwrap_err();
        assert!(
            detail.contains("not listening"),
            "unexpected detail: {detail}"
        );
    }

    #[test]
    fn the_probe_requires_an_explicit_port() {
        // A portless URL must fail; which resolver complaint it raises is
        // not part of the contract.
        assert!(probe_sidecar("http://127.0.0.1", Duration::from_millis(250)).is_err());
    }

    #[test]
    fn the_probe_rejects_a_non_url() {
        assert!(probe_sidecar("127.0.0.1:8000", Duration::from_millis(250)).is_err());
    }
}
