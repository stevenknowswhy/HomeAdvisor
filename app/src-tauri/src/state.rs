//! The application state: the Rust core's handles, held where the webview
//! cannot reach them.
//!
//! `AppState` is the only object the IPC commands touch. It owns the
//! encrypted store (spec, milestone 2: "the Rust core keeps sole ownership
//! of the store, the gate, and the sidecar; the frontend renders state,
//! never data access"), the loopback scan client, and the sidecar
//! supervisor. Nothing here is serializable or clonable into the webview
//! on purpose.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use ha_privacy::{probe_health, LayaSidecar};
use ha_store::Store;
use tauri::Manager as _;

use crate::commands::AppError;
use crate::packs::{LadderSink, PluginNotifier};
use crate::supervisor::{
    CommandSpawner, ExternalSidecar, HealthProbe, HttpHealthProbe, SidecarSpawner,
    SidecarSupervisor, SupervisionPolicy, HEALTH_CHECK_INTERVAL,
};

/// How long the privacy-status probe waits for the sidecar's `/health`
/// answer before declaring it unavailable. Bounded and short: the probe
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
    /// so this handle can only ever point at this machine.
    sidecar: LayaSidecar,
    /// The sidecar supervisor (spawn, health-check, restart) — the machine
    /// behind `privacy_status` and the gated egress pre-flight.
    supervisor: SidecarSupervisor,
    /// The local notification sink — `None` in tests and headless runs,
    /// the Tauri plugin in production (set in [`open_state`]).
    notifier: Option<Arc<dyn LadderSink>>,
}

impl AppState {
    /// Build the app state and begin supervising the sidecar. The
    /// supervisor's own first tick decides between `Healthy` and
    /// `Degraded` — either way the status is honest from the first read.
    pub fn new(store: Store, sidecar: LayaSidecar, supervisor: SidecarSupervisor) -> Self {
        supervisor.start();
        Self {
            store: Mutex::new(store),
            sidecar,
            supervisor,
            notifier: None,
        }
    }

    /// Attach the notification sink — the production path, called from
    /// [`open_state`] where the app handle lives.
    pub(crate) fn with_notifier(mut self, notifier: Arc<dyn LadderSink>) -> Self {
        self.notifier = Some(notifier);
        self
    }

    /// The notification sink, when this run has one.
    pub(crate) fn notifier(&self) -> Option<Arc<dyn LadderSink>> {
        self.notifier.clone()
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

    pub(crate) fn supervisor(&self) -> &SidecarSupervisor {
        &self.supervisor
    }

    /// Kill any supervised sidecar. The app calls this on exit: a closed
    /// window must not orphan the child process it spawned.
    pub(crate) fn shutdown(&self) {
        self.supervisor.stop();
    }
}

/// Open the state the running app needs: the store from the configured
/// location, the scan client from the configured loopback endpoint, and the
/// sidecar supervisor.
///
/// Sidecar configuration:
///
/// - `HOMEADVISOR_SIDECAR_URL` — the loopback endpoint the scan client and
///   the health probe target (default: [`DEFAULT_SIDECAR_URL`]).
/// - `HOMEADVISOR_SIDECAR_COMMAND` — when set, the app spawns that command
///   as the sidecar and restarts it when it dies or stops answering
///   (supervised mode). The documented sidecar is upstream's `laya-serve`
///   (`pip install "laya[serve]"`, exact version pinned in
///   `crates/ha-privacy/MANUAL-SMOKE.md`). The child inherits this process's
///   environment, so launch it with the runbook's pins — `LAYA_HOST=127.0.0.1`
///   is the security-relevant one, because laya-serve's bare-metal default
///   binds `0.0.0.0`. When unset, the sidecar is managed outside the app
///   (external mode): the supervisor probes and reports but cannot respawn.
///   Either way, fail-closed is the contract.
///
/// Fail-closed at startup: without a database key the app cannot serve
/// reads, so it aborts with a diagnostic instead of opening a plaintext or
/// empty store. The key machine lives in [`crate::keystore`]:
/// `HOMEADVISOR_DB_KEY` overrides everything when set (the same hand-off
/// the CLI documents — tests and CLI interop rely on it unchanged);
/// otherwise macOS keys the store from the login Keychain, first run
/// generating the key and later runs reading it back. A database whose key
/// is missing or unreadable stops startup with a plain-language diagnostic.
///
/// Called before the event loop (`lib.rs`) so every failure here exits
/// cleanly with its cause printed — never an unwind across the launch
/// callback.
pub fn open_state(app: &tauri::AppHandle) -> Result<AppState, Box<dyn std::error::Error>> {
    let db_path = match std::env::var("HOMEADVISOR_DB") {
        Ok(path) => std::path::PathBuf::from(path),
        Err(_) => {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            dir.join("homeadvisor.db")
        }
    };
    // Read before anything opens the store: the key machine decides between
    // fresh-create and reopen on exactly this fact.
    let db_exists = db_path.exists();

    // The override wins when set, with its error paths intact (empty or
    // non-unicode values fail as they always did). An absent variable is no
    // longer an error: the keystore machine takes over.
    let env_key = match std::env::var("HOMEADVISOR_DB_KEY") {
        Ok(key) => Some(key),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => {
            return Err(Box::new(
                crate::keystore::StartupKeyError::InvalidOverride {
                    detail: error.to_string(),
                },
            ));
        }
    };

    let key = crate::keystore::resolve_startup_key(
        &db_path,
        db_exists,
        env_key,
        crate::keystore::platform_keystore().as_deref(),
        crate::keystore::generate_key,
    )?;
    let store = Store::open(db_path, &key)?;

    let sidecar_url = std::env::var("HOMEADVISOR_SIDECAR_URL")
        .unwrap_or_else(|_| DEFAULT_SIDECAR_URL.to_string());
    let sidecar = LayaSidecar::new(&sidecar_url)?;

    let spawner: Box<dyn SidecarSpawner> = match std::env::var("HOMEADVISOR_SIDECAR_COMMAND") {
        Ok(command) if !command.trim().is_empty() => {
            let argv = shell_words::split(&command)
                .map_err(|error| format!("could not parse HOMEADVISOR_SIDECAR_COMMAND: {error}"))?;
            Box::new(CommandSpawner::new(argv))
        }
        _ => Box::new(ExternalSidecar),
    };
    let probe: Box<dyn HealthProbe> =
        Box::new(HttpHealthProbe::new(&sidecar_url, SIDECAR_PROBE_TIMEOUT));
    let supervisor = SidecarSupervisor::with_policy(probe, spawner, SupervisionPolicy::default());

    let state = AppState::new(store, sidecar, supervisor)
        // The notification sink is production-only: local notifications
        // through the Tauri plugin, permission requested at the first
        // evaluation that sends.
        .with_notifier(Arc::new(PluginNotifier::new(app.clone())));

    // The startup re-evaluation: idempotent via the budget row (a same-day
    // run writes nothing and fires no rung). Non-fatal by design — the
    // app opens even when the advice packs fail; the daily view renders
    // its empty state and the failure is logged.
    crate::packs::run_daily_evaluation(&state);

    // The background loop keeps restarts happening without a status read
    // asking for one; the supervisor supersedes any earlier loop.
    state
        .supervisor()
        .start_health_loop(HEALTH_CHECK_INTERVAL)?;
    Ok(state)
}

/// Is the sidecar healthy right now?
///
/// A bounded `GET /health` — a probe, not a scan: it costs no forward pass
/// and answers exactly one question, "is the laya-serve sidecar answering
/// with its checkpoints loaded". A listening-but-hung sidecar — the failure
/// mode the TCP connect probe this replaced could not see — times out here
/// like any other unavailability. The real scan pipeline (and its failure
/// mapping) is tested in `ha-privacy`.
pub fn probe_sidecar(base_url: &str, timeout: Duration) -> Result<(), String> {
    require_explicit_port(base_url)?;
    probe_health(base_url, timeout)
}

/// A URL without an explicit port is a configuration error: the probe
/// guesses nothing about which port a sidecar process picked (and probing
/// port 80 by accident would be a lie).
fn require_explicit_port(base_url: &str) -> Result<(), String> {
    let host_port = endpoint(base_url)?;
    let port = host_port.rsplit_once(':').map(|(_, port)| port);
    if port.map(|port| port.parse::<u16>().is_ok()) != Some(true) {
        return Err(format!(
            "{base_url} has no explicit port — the probe guesses nothing about which port a sidecar picked"
        ));
    }
    Ok(())
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

    /// A minimal `/health` responder: accepts connections and answers each
    /// with laya-serve's healthy body. Returns the base URL to probe.
    fn healthy_sidecar() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(stream) => stream,
                    Err(_) => break,
                };
                // Read whatever arrived; a probe is tiny and lands in one
                // segment. Not read to parse — just to give the client's
                // write somewhere to go before the answer.
                let mut buf = [0u8; 4096];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                let body = "{\"status\":\"ok\"}";
                let _ = std::io::Write::write_all(
                    &mut stream,
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                );
            }
        });
        url
    }

    #[test]
    fn the_probe_reaches_a_healthy_loopback_sidecar() {
        let base_url = healthy_sidecar();
        assert!(probe_sidecar(&base_url, Duration::from_millis(750)).is_ok());
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
            detail.contains("could not reach the sidecar"),
            "unexpected detail: {detail}"
        );
    }

    #[test]
    fn the_probe_requires_an_explicit_port() {
        // A portless URL must fail — an accidental probe of port 80 would
        // be a lie; which resolver complaint it raises is not the contract.
        assert!(probe_sidecar("http://127.0.0.1", Duration::from_millis(250)).is_err());
    }

    #[test]
    fn the_probe_rejects_a_non_url() {
        assert!(probe_sidecar("127.0.0.1:8000", Duration::from_millis(250)).is_err());
    }
}
