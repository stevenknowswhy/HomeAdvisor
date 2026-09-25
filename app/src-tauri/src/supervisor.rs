//! Sidecar supervision: the app owns the Laya sidecar's lifecycle.
//!
//! Spec, milestone 2 ("What the app does — first slice"): the app spawns and
//! health-checks the Laya loopback sidecar, restarts it on failure, and
//! reports its state in the UI; when it is down, any gated operation
//! surfaces BLOCK — fail-closed survives the surface.
//!
//! The state machine:
//!
//! ```text
//!      start()            probe ok            probe ok
//!   ────────────▶ Starting ───────────▶ Healthy ───────▶ Healthy
//!                    │                                  (recovers from a
//!                    │ probe fails                       transient failure
//!                    ▼ without a restart)
//!              Degraded ──▶ restart when the child is dead, the start
//!                    ▲       window expired, or the failure budget is
//!                    └─────── spent; spawn failure lands here too
//! ```
//!
//! Two supervision modes fall out of the spawner, not the machine:
//!
//! - **Supervised** — `HOMEADVISOR_SIDECAR_COMMAND` names the sidecar launch
//!   command; the app spawns the child, and the machine kills and respawns
//!   it when it dies or stops answering.
//! - **External** — nothing is configured (a family running the sidecar
//!   themselves, or development against a manually started one); the
//!   supervisor probes and reports but cannot respawn. Either way the
//!   fail-closed contract is the same: an unhealthy supervisor blocks every
//!   gated operation (see `crate::egress`) and `privacy_status` reports
//!   unavailable.
//!
//! All state sits behind one mutex, and every transition is a total match
//! over the state — the same plain-code discipline as the `ha-core` router.
//! The lock is held across a probe, which is bounded by the probe timeout:
//! one probe at a time, and a status read can never see a half-applied
//! transition. A poisoned lock recovers via `into_inner` rather than
//! cascading: probes are `Result`-returning by contract, so a panic means a
//! bug in a component, and the next tick re-derives the state — a bricked
//! status screen is not fail-closed, it is fail-blind.

use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::state::probe_sidecar;

/// How often the background health-check loop probes. Status reads probe on
/// demand regardless; the loop exists so restarts happen without one.
pub const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(10);

/// How long `Starting` waits for the sidecar to answer its first probe
/// before the supervisor declares the start a failure and respawns. The real
/// sidecar loads its model for 25–35 s before the port answers
/// (`ha-privacy/MANUAL-SMOKE.md`), so this must clear that window.
const DEFAULT_START_TIMEOUT: Duration = Duration::from_secs(90);

/// Consecutive failed probes that turn a previously healthy sidecar into a
/// restart. One probe failure is a transient blip; two in a row is a sidecar
/// that has stopped answering.
const DEFAULT_RESTART_AFTER_FAILURES: u32 = 2;

/// Tunable timings. Production uses [`SupervisionPolicy::default`]; tests
/// shrink the windows to keep the suite fast. The per-probe connect timeout
/// is not here — the probe owns its own timeout (one source of truth).
#[derive(Debug, Clone)]
pub struct SupervisionPolicy {
    /// How long `Starting` may last before the start is abandoned and
    /// retried.
    pub start_timeout: Duration,
    /// Consecutive failed probes (on a live child) that trigger a restart.
    pub restart_after_failures: u32,
}

impl Default for SupervisionPolicy {
    fn default() -> Self {
        Self {
            start_timeout: DEFAULT_START_TIMEOUT,
            restart_after_failures: DEFAULT_RESTART_AFTER_FAILURES,
        }
    }
}

/// The sidecar lifecycle state. The UI and the gated egress path read this;
/// `privacy_status` maps it onto the family-facing status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarState {
    /// Supervision is not running — before `start` or after `stop`. No child
    /// exists and no probe changes anything.
    Stopped,
    /// A child was spawned (or re-spawned) and has not yet answered a probe.
    /// The real sidecar spends 25–35 s here on a cold model load.
    Starting { since: Instant },
    /// The last probe passed; the scan layer can do its job.
    Healthy,
    /// The last probe(s) failed. The detail is the probe's own complaint.
    Degraded { failures: u32, detail: String },
}

/// A supervised child process. `is_alive` is expected to reap exited
/// children as a side effect (a supervisor that leaks zombies is a leak).
pub trait SidecarProcess: Send {
    fn is_alive(&mut self) -> bool;
    /// Kill and reap. Idempotent; a child already gone is a no-op.
    fn kill(&mut self);
}

/// The production process handle: a real spawned child, killed and reaped on
/// drop so a crashing app cannot orphan the sidecar.
pub struct ChildProcess(std::process::Child);

impl SidecarProcess for ChildProcess {
    fn is_alive(&mut self) -> bool {
        // `try_wait` reaps on exit; a wait error means unknown — report
        // alive and let the probe decide rather than killing on a guess.
        self.0.try_wait().map_or(true, |status| status.is_none())
    }

    fn kill(&mut self) {
        // Kill then reap: without the wait the child stays a zombie.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Creates supervised children. `spawn` must be cheap and must not block:
/// it runs while the supervisor's lock is held.
pub trait SidecarSpawner: Send {
    fn spawn(&self) -> Result<Box<dyn SidecarProcess>, String>;
}

/// The supervised mode: launch the configured command line. The child
/// inherits this process's environment — the sidecar needs nothing else.
#[derive(Debug, Clone)]
pub struct CommandSpawner {
    argv: Vec<String>,
}

impl CommandSpawner {
    pub fn new(argv: Vec<String>) -> Self {
        Self { argv }
    }
}

impl SidecarSpawner for CommandSpawner {
    fn spawn(&self) -> Result<Box<dyn SidecarProcess>, String> {
        let (program, args) = self
            .argv
            .split_first()
            .ok_or_else(|| "the sidecar command is empty".to_string())?;
        Command::new(program)
            .args(args)
            .spawn()
            .map(|child| Box::new(ChildProcess(child)) as Box<dyn SidecarProcess>)
            .map_err(|error| format!("could not spawn the sidecar command: {error}"))
    }
}

/// The external mode: the sidecar is managed outside this process. Spawn
/// always fails — the supervisor never holds a child here, so ticks report
/// the probe's own complaint (see `tick_locked`) instead of attempting
/// restarts that cannot help.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExternalSidecar;

impl SidecarSpawner for ExternalSidecar {
    fn spawn(&self) -> Result<Box<dyn SidecarProcess>, String> {
        Err(
            "no sidecar command is configured — set HOMEADVISOR_SIDECAR_COMMAND to let the app \
             supervise the sidecar process"
                .to_string(),
        )
    }
}

/// Answers one question: is the sidecar endpoint up right now?
pub trait HealthProbe: Send {
    fn probe(&self) -> Result<(), String>;
}

/// The production probe: a bounded TCP connect to the loopback endpoint the
/// scan client is configured for — the same probe `state.rs` tests.
#[derive(Debug, Clone)]
pub struct TcpProbe {
    base_url: String,
    timeout: Duration,
}

impl TcpProbe {
    pub fn new(base_url: &str, timeout: Duration) -> Self {
        Self {
            base_url: base_url.to_string(),
            timeout,
        }
    }
}

impl HealthProbe for TcpProbe {
    fn probe(&self) -> Result<(), String> {
        probe_sidecar(&self.base_url, self.timeout)
    }
}

/// The supervisor handle. Clonable — the health-check loop, the IPC
/// commands, and the gated egress path all hold one.
#[derive(Clone)]
pub struct SidecarSupervisor {
    inner: Arc<Mutex<SupervisorInner>>,
}

struct SupervisorInner {
    policy: SupervisionPolicy,
    probe: Box<dyn HealthProbe>,
    spawner: Box<dyn SidecarSpawner>,
    process: Option<Box<dyn SidecarProcess>>,
    state: SidecarState,
    /// Successful spawns so far — the initial one plus every restart. The
    /// receipt-free answer to "is this thing thrashing?".
    spawns: u64,
    /// Bumped on every `start_health_loop` and `stop`; a loop thread whose
    /// generation no longer matches exits on its next wake.
    loop_generation: u64,
}

impl SidecarSupervisor {
    pub fn with_policy(
        probe: Box<dyn HealthProbe>,
        spawner: Box<dyn SidecarSpawner>,
        policy: SupervisionPolicy,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SupervisorInner {
                policy,
                probe,
                spawner,
                process: None,
                state: SidecarState::Stopped,
                spawns: 0,
                loop_generation: 0,
            })),
        }
    }

    /// Begin supervising: spawn the sidecar if a spawner is configured,
    /// probe immediately, and leave the machine in the state the probe
    /// found. Idempotent — a running supervisor is left alone.
    pub fn start(&self) -> SidecarState {
        let mut inner = self.lock();
        if !matches!(inner.state, SidecarState::Stopped) {
            return inner.state.clone();
        }
        match inner.spawner.spawn() {
            Ok(child) => {
                inner.process = Some(child);
                inner.spawns += 1;
                inner.state = SidecarState::Starting {
                    since: Instant::now(),
                };
            }
            Err(detail) => {
                // External mode, or a failed first spawn: degraded until a
                // probe (or a later respawn attempt) says otherwise.
                inner.state = SidecarState::Degraded {
                    failures: 0,
                    detail,
                };
            }
        }
        tick_locked(&mut inner)
    }

    /// Stop supervising: detach any health-check loop, kill the child, and
    /// park at [`SidecarState::Stopped`]. Gated operations fail closed on a
    /// stopped supervisor — stopping it is a deliberate act.
    pub fn stop(&self) {
        let mut inner = self.lock();
        inner.loop_generation += 1;
        if let Some(mut process) = inner.process.take() {
            process.kill();
        }
        inner.state = SidecarState::Stopped;
    }

    /// One health check, and whatever transition it earns. Restarting a
    /// sidecar is the point of this machine, so a tick that finds the child
    /// dead respawns it here and now.
    pub fn tick(&self) -> SidecarState {
        let mut inner = self.lock();
        tick_locked(&mut inner)
    }

    /// The pre-flight check for the gated egress path: tick, then demand
    /// [`SidecarState::Healthy`]. Anything else is a bounded, actionable
    /// refusal — the gate must not stall waiting for a booting sidecar, and
    /// it must certainly not send.
    pub fn ensure_healthy(&self) -> Result<(), String> {
        match self.tick() {
            SidecarState::Healthy => Ok(()),
            SidecarState::Starting { .. } => Err(
                "the sidecar is starting — gated operations stay BLOCKED until it is healthy"
                    .to_string(),
            ),
            SidecarState::Degraded { detail, .. } => Err(detail),
            SidecarState::Stopped => Err("sidecar supervision is stopped".to_string()),
        }
    }

    /// The current state, without probing. Test-and-diagnostics access:
    /// the UI reads `privacy_status`, which answers the same question
    /// through `ensure_healthy`. #[cfg(test)] — the machine's own suite and
    /// the egress fail-closed suite drive ticks and assert on it.
    #[cfg(test)]
    pub(crate) fn state(&self) -> SidecarState {
        self.lock().state.clone()
    }

    /// Successful spawns — the initial one plus restarts. #[cfg(test)] —
    /// the suites assert spawn counts; no production surface reads it yet.
    #[cfg(test)]
    pub(crate) fn spawn_count(&self) -> u64 {
        self.lock().spawns
    }

    /// Run the health-check loop on a background thread: probe, transition,
    /// and restart failures without anyone asking. Returns when the thread
    /// is running; `stop` retires it. A second call supersedes the first
    /// loop rather than stacking a second one.
    pub fn start_health_loop(&self, interval: Duration) -> std::io::Result<()> {
        let generation = {
            let mut inner = self.lock();
            inner.loop_generation += 1;
            inner.loop_generation
        };
        let supervisor = self.clone();
        std::thread::Builder::new()
            .name("sidecar-supervision".to_string())
            .spawn(move || loop {
                {
                    let inner = supervisor.lock();
                    if inner.loop_generation != generation {
                        break;
                    }
                }
                std::thread::sleep(interval);
                supervisor.tick();
            })?;
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, SupervisorInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Test seam: kill the current child without touching the state machine,
    /// simulating a sidecar crash. The next failing tick observes the dead
    /// child and restarts. #[cfg(test)] — and crate-visible so the egress
    /// fail-closed suite can drive real kill/restart cycles.
    #[cfg(test)]
    pub(crate) fn test_kill_child(&self) {
        let mut inner = self.lock();
        if let Some(mut process) = inner.process.take() {
            process.kill();
        }
    }
}

/// The transition table, applied under the lock. Total over the state: every
/// state has an answer to every probe outcome, and none of them is "send".
fn tick_locked(inner: &mut SupervisorInner) -> SidecarState {
    if matches!(inner.state, SidecarState::Stopped) {
        return inner.state.clone();
    }

    match inner.probe.probe() {
        Ok(()) => {
            // From any running state: the sidecar is answering. A degraded
            // one recovers without a restart; a starting one passes its
            // first check.
            inner.state = SidecarState::Healthy;
        }
        Err(detail) => {
            let child_alive = inner
                .process
                .as_mut()
                .is_some_and(|process| process.is_alive());
            // External mode: nothing was ever spawnable — there is no child
            // to restart, and the probe's own complaint is the actionable
            // detail (a respawn attempt would just overwrite it with the
            // spawner's configuration error).
            if inner.spawns == 0 && !child_alive {
                let failures = match &inner.state {
                    SidecarState::Degraded { failures, .. } => failures + 1,
                    SidecarState::Healthy | SidecarState::Starting { .. } => 1,
                    SidecarState::Stopped => unreachable!("stopped is handled at the top"),
                };
                inner.state = SidecarState::Degraded { failures, detail };
                return inner.state.clone();
            }
            match &inner.state {
                SidecarState::Healthy => {
                    if child_alive {
                        inner.state = SidecarState::Degraded {
                            failures: 1,
                            detail,
                        };
                    } else {
                        // The process is gone — nothing to wait out.
                        restart_locked(inner);
                    }
                }
                SidecarState::Starting { since } => {
                    // A cold model load takes tens of seconds; a starting
                    // sidecar is patience, not failure — until the start
                    // window expires or the child dies.
                    let expired = since.elapsed() >= inner.policy.start_timeout;
                    if !child_alive || expired {
                        restart_locked(inner);
                    }
                }
                SidecarState::Degraded { failures, .. } => {
                    let spent = *failures + 1 >= inner.policy.restart_after_failures;
                    if !child_alive || spent {
                        restart_locked(inner);
                    } else {
                        inner.state = SidecarState::Degraded {
                            failures: failures + 1,
                            detail,
                        };
                    }
                }
                SidecarState::Stopped => unreachable!("stopped is handled at the top"),
            }
        }
    }
    inner.state.clone()
}

/// Kill whatever is running and try again. A failed spawn degrades with the
/// spawn error as the detail; the next failing tick retries (the budget is
/// pre-spent so a failing spawn is retried every tick, not accumulated
/// toward).
fn restart_locked(inner: &mut SupervisorInner) {
    if let Some(mut process) = inner.process.take() {
        process.kill();
    }
    match inner.spawner.spawn() {
        Ok(child) => {
            inner.process = Some(child);
            inner.spawns += 1;
            inner.state = SidecarState::Starting {
                since: Instant::now(),
            };
        }
        Err(detail) => {
            inner.state = SidecarState::Degraded {
                failures: inner.policy.restart_after_failures,
                detail,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A probe from a scripted sequence: pops results until the script is
    /// exhausted, then repeats the last result (an empty script always
    /// succeeds). Returns a concrete type so call sites' `Box::new(...)`
    /// coerces to the trait object.
    fn scripted_probe(script: Vec<Result<(), String>>) -> impl HealthProbe {
        /// The remaining script plus the last result it delivered (for
        /// repeating after exhaustion).
        type ProbeScript = (VecDeque<Result<(), String>>, Option<Result<(), String>>);
        struct Scripted(Arc<Mutex<ProbeScript>>);
        impl HealthProbe for Scripted {
            fn probe(&self) -> Result<(), String> {
                let mut state = self.0.lock().unwrap();
                match state.0.pop_front() {
                    Some(result) => {
                        state.1 = Some(result.clone());
                        result
                    }
                    None => state.1.clone().unwrap_or(Ok(())),
                }
            }
        }
        Scripted(Arc::new(Mutex::new((script.into(), None))))
    }

    fn err(detail: &str) -> Result<(), String> {
        Err(detail.to_string())
    }

    /// A fake child that records whether it was killed. Shared flags let the
    /// test reach into the current process without a downcast.
    struct FakeProcess {
        alive: Arc<AtomicBool>,
        killed: Arc<AtomicBool>,
    }

    impl SidecarProcess for FakeProcess {
        fn is_alive(&mut self) -> bool {
            self.alive.load(Ordering::Relaxed)
        }

        fn kill(&mut self) {
            self.alive.store(false, Ordering::Relaxed);
            self.killed.store(true, Ordering::Relaxed);
        }
    }

    /// A fake spawner: every spawn yields a live fake child. The shared
    /// records let tests assert spawn and kill counts.
    #[derive(Clone)]
    struct FakeSpawner {
        spawns: Arc<Mutex<Vec<Arc<AtomicBool>>>>,
    }

    impl FakeSpawner {
        fn new() -> Self {
            Self {
                spawns: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn spawn_count(&self) -> u64 {
            self.spawns.lock().unwrap().len() as u64
        }
    }

    impl SidecarSpawner for FakeSpawner {
        fn spawn(&self) -> Result<Box<dyn SidecarProcess>, String> {
            let alive = Arc::new(AtomicBool::new(true));
            self.spawns.lock().unwrap().push(alive.clone());
            Ok(Box::new(FakeProcess {
                alive,
                killed: Arc::new(AtomicBool::new(false)),
            }))
        }
    }

    fn fast_policy() -> SupervisionPolicy {
        SupervisionPolicy {
            start_timeout: Duration::from_millis(50),
            restart_after_failures: 2,
        }
    }

    fn wait_for_healthy(supervisor: &SidecarSupervisor, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            if supervisor.ensure_healthy().is_ok() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the sidecar never became healthy"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn start_spawns_the_sidecar_and_a_passing_probe_marks_it_healthy() {
        let spawner = FakeSpawner::new();
        let supervisor = SidecarSupervisor::with_policy(
            Box::new(scripted_probe(vec![])),
            Box::new(spawner.clone()),
            fast_policy(),
        );

        let state = supervisor.start();
        assert_eq!(state, SidecarState::Healthy);
        assert_eq!(supervisor.spawn_count(), 1);
        assert_eq!(spawner.spawn_count(), 1);
    }

    #[test]
    fn an_unconfigured_spawner_degrades_and_never_reads_healthy_while_down() {
        let supervisor = SidecarSupervisor::with_policy(
            Box::new(scripted_probe(vec![err("not listening")])),
            Box::new(ExternalSidecar),
            fast_policy(),
        );

        let state = supervisor.start();
        // External mode: the probe's own complaint is the actionable detail —
        // the missing spawn command is not something a tick can fix.
        assert!(
            matches!(&state, SidecarState::Degraded { detail, .. } if detail.contains("not listening")),
            "unexpected state: {state:?}"
        );
        // Ticks keep reporting degraded: fail-closed.
        assert!(matches!(supervisor.tick(), SidecarState::Degraded { .. }));
        assert!(supervisor.ensure_healthy().is_err());
    }

    #[test]
    fn a_transient_probe_failure_degrades_and_recovers_without_a_restart() {
        let spawner = FakeSpawner::new();
        let supervisor = SidecarSupervisor::with_policy(
            Box::new(scripted_probe(vec![Ok(()), err("probe timeout"), Ok(())])),
            Box::new(spawner.clone()),
            fast_policy(),
        );
        assert_eq!(supervisor.start(), SidecarState::Healthy);

        assert!(matches!(
            supervisor.tick(),
            SidecarState::Degraded { failures: 1, .. }
        ));
        assert_eq!(supervisor.tick(), SidecarState::Healthy);
        assert_eq!(
            spawner.spawn_count(),
            1,
            "a recovered sidecar must not have been restarted"
        );
    }

    #[test]
    fn the_failure_budget_restarts_a_live_but_unresponsive_sidecar() {
        let spawner = FakeSpawner::new();
        let supervisor = SidecarSupervisor::with_policy(
            Box::new(scripted_probe(vec![
                Ok(()),
                err("probe timeout"),
                err("probe timeout"),
                Ok(()),
            ])),
            Box::new(spawner.clone()),
            fast_policy(),
        );
        assert_eq!(supervisor.start(), SidecarState::Healthy);

        // Second consecutive failure spends the budget → respawn.
        assert!(matches!(
            supervisor.tick(),
            SidecarState::Degraded { failures: 1, .. }
        ));
        assert!(matches!(supervisor.tick(), SidecarState::Starting { .. }));
        assert_eq!(spawner.spawn_count(), 2);
        // The replacement answers → healthy again.
        assert_eq!(supervisor.tick(), SidecarState::Healthy);
    }

    #[test]
    fn a_dead_child_restarts_on_the_first_failing_probe() {
        let spawner = FakeSpawner::new();
        let supervisor = SidecarSupervisor::with_policy(
            Box::new(scripted_probe(vec![
                Ok(()),
                err("connection refused"),
                Ok(()),
            ])),
            Box::new(spawner.clone()),
            fast_policy(),
        );
        assert_eq!(supervisor.start(), SidecarState::Healthy);

        supervisor.test_kill_child();
        // One tick is enough: a dead child is not waited out.
        assert!(matches!(supervisor.tick(), SidecarState::Starting { .. }));
        assert_eq!(spawner.spawn_count(), 2);
        assert_eq!(supervisor.tick(), SidecarState::Healthy);
    }

    #[test]
    fn a_start_that_never_becomes_healthy_is_retried_after_the_start_window() {
        // Every probe fails and the child stays alive (listening but not
        // answering — the probe's complaint). The start window expires and
        // the supervisor respawns rather than waiting forever.
        let spawner = FakeSpawner::new();
        let supervisor = SidecarSupervisor::with_policy(
            Box::new(scripted_probe(vec![
                err("not listening"),
                err("not listening"),
                err("not listening"),
            ])),
            Box::new(spawner.clone()),
            fast_policy(),
        );

        // start(): spawn succeeds → Starting; the immediate tick fails but
        // the start window (50 ms) has not run out yet.
        assert!(matches!(supervisor.start(), SidecarState::Starting { .. }));
        std::thread::sleep(Duration::from_millis(60));
        assert!(matches!(supervisor.tick(), SidecarState::Starting { .. }));
        assert_eq!(spawner.spawn_count(), 2, "the expired start was retried");
    }

    #[test]
    fn stop_kills_the_child_and_parks_the_machine() {
        let spawner = FakeSpawner::new();
        let supervisor = SidecarSupervisor::with_policy(
            Box::new(scripted_probe(vec![Ok(()), Ok(())])),
            Box::new(spawner.clone()),
            fast_policy(),
        );
        supervisor.start();
        assert_eq!(supervisor.state(), SidecarState::Healthy);

        supervisor.stop();
        assert_eq!(supervisor.state(), SidecarState::Stopped);
        // Ticks are no-ops while stopped, and the pre-flight refuses.
        assert_eq!(supervisor.tick(), SidecarState::Stopped);
        assert!(supervisor.ensure_healthy().is_err());
        assert_eq!(
            spawner.spawn_count(),
            1,
            "a stopped supervisor must not spawn"
        );
    }

    #[test]
    fn ensure_healthy_names_the_starting_state_as_the_block_reason() {
        let spawner = FakeSpawner::new();
        // Both probes fail: the start's own tick, then ensure_healthy's. The
        // child is alive and the start window has not expired, so the machine
        // stays Starting — and that is what the refusal must say.
        let supervisor = SidecarSupervisor::with_policy(
            Box::new(scripted_probe(vec![
                err("not listening"),
                err("not listening"),
            ])),
            Box::new(spawner),
            fast_policy(),
        );
        supervisor.start();

        let detail = supervisor.ensure_healthy().unwrap_err();
        assert!(
            detail.contains("starting"),
            "the detail should say the sidecar is starting: {detail}"
        );
    }

    #[test]
    fn the_health_loop_restarts_a_dead_sidecar_without_manual_ticks() {
        // Loop ticks: start's probe(Ok), then Err, Err (budget spent →
        // restart), then Ok (replacement healthy).
        let spawner = FakeSpawner::new();
        let supervisor = SidecarSupervisor::with_policy(
            Box::new(scripted_probe(vec![
                Ok(()),
                err("probe timeout"),
                err("probe timeout"),
                Ok(()),
            ])),
            Box::new(spawner.clone()),
            fast_policy(),
        );
        assert_eq!(supervisor.start(), SidecarState::Healthy);

        supervisor
            .start_health_loop(Duration::from_millis(30))
            .unwrap();
        wait_for_healthy(&supervisor, Duration::from_secs(5));
        assert_eq!(
            spawner.spawn_count(),
            2,
            "the loop must have restarted the sidecar"
        );
        supervisor.stop();
    }

    /// Real process semantics: the supervised child is a genuine OS process,
    /// killed and respawned by the machine.
    #[test]
    fn a_killed_real_child_is_respawned() {
        let up = Arc::new(AtomicBool::new(true));
        struct FlagProbe(Arc<AtomicBool>);
        impl HealthProbe for FlagProbe {
            fn probe(&self) -> Result<(), String> {
                if self.0.load(Ordering::Relaxed) {
                    Ok(())
                } else {
                    Err("simulated outage".to_string())
                }
            }
        }
        let supervisor = SidecarSupervisor::with_policy(
            Box::new(FlagProbe(up.clone())),
            Box::new(CommandSpawner::new(vec![
                "sleep".to_string(),
                "300".to_string(),
            ])),
            fast_policy(),
        );
        assert_eq!(supervisor.start(), SidecarState::Healthy);
        assert_eq!(supervisor.spawn_count(), 1);

        // The child crashes and the probe goes dark: one tick restarts.
        supervisor.test_kill_child();
        up.store(false, Ordering::Relaxed);
        assert!(matches!(supervisor.tick(), SidecarState::Starting { .. }));
        assert_eq!(supervisor.spawn_count(), 2);

        // The replacement comes up; stop cleans it up.
        up.store(true, Ordering::Relaxed);
        assert_eq!(supervisor.tick(), SidecarState::Healthy);
        supervisor.stop();
        assert_eq!(supervisor.state(), SidecarState::Stopped);
    }
}
