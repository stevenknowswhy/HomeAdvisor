# Outbound threat model

The threat model for Home Advisor's outbound stack, as milestone 2 requires (spec,
"Threat model document"). It enumerates every path a byte can take off the device,
states the control on each path, and states the residual risk honestly — the
external privacy claim is written to cover exactly what this document defends,
and no further.

Status: describes the tree at `main` after the Tauri 2 shell (PR #7). Where a path
is designed but not yet exercised by shipped code, that is said explicitly.

## The claim, and exactly how far it reaches

**The claim:** *"The app never sends your PII anywhere — every outbound byte is
gated, redacted, and receipted."*

**Scope of the claim:** the app's own egress. Every outbound path the app's code
can originate is enumerated below and is either gated through the privacy
pipeline, bound to the loopback interface, user-initiated, or enforced off by
policy in CI.

**The declared boundary (outside the claim):** the third-party webview runtime.
On Windows the app renders in Microsoft's WebView2 runtime, which ships with
telemetry enabled by default; equivalent third-party browser runtimes render the
UI on Linux (WebKitGTK) and macOS (WKWebView). No app-level gate can audit or
constrain the runtime vendor's own traffic. This is the reason the earlier
device-level claim — "no PII ever leaves the computer" — was retired (spec,
"Resolved decisions — September 24, 2026"): an honest threat model names the
runtime as a known, declared boundary instead of hiding it behind an absolute.

**Assumption, stated once:** no malware already running on the user's machine is
in scope. An on-device attacker can read the SQLCipher database's memory, the
OS keystore that holds its key (on macOS, the login Keychain — and losing that
Keychain item makes the data permanently undecryptable by design), and the
loopback port, and defeats any local-first design by definition. Local device
security is a separate threat class (disk encryption, OS account security)
that this document does not claim to solve.

## Outbound path inventory

Every path a byte can leave the device, enumerated. "Send" means: information
originated by the app crosses a network boundary to a destination the user did
not explicitly choose.

| # | Path                                  | Crosses a network? | Control                                                  | Claim covered? |
|---|---------------------------------------|--------------------|----------------------------------------------------------|----------------|
| 1 | Gated research egress                 | External send      | Privacy gate: redact → scan → decide; receipt per attempt | Yes            |
| 2 | Laya leak-scan (sidecar)              | Loopback only      | Loopback-enforced client; Layer 1 output only            | Yes            |
| 3 | Laya model download                   | External fetch     | User-initiated, outside the app binary                   | Yes            |
| 4 | Tauri IPC                             | No — in-process    | No port exists; enumerated command surface               | Yes            |
| 5 | Webview runtime                       | Vendor-dependent   | **None — declared boundary** (WebView2 telemetry named)  | No — declared  |
| 6 | Updates                               | External fetch     | User-initiated only; no updater, no update check         | Yes            |
| 7 | Dependencies (supply chain)           | Whatever a dep does| Zero-telemetry policy enforced by `cargo-deny` in CI     | Yes            |

There are no other paths. In particular: no HTTP listener exists (Tauri IPC
replaces it), no analytics or crash-reporting SDK is present, and the CI log of
the dependency audit is the receipt for that last sentence.

## The paths in detail

### 1. Gated research egress — the only sanctioned external send

When the app (in a later milestone, research agents) needs data from the outside
world, the payload follows one road, implemented in `crates/ha-privacy`:

```
construct → redact (Layer 1) → scan (Layer 2) → decide (Layer 3) → record → send
```

- **Layer 1 — deterministic redaction** (`src/redact.rs`): allowlist-based.
  Exact ages, incomes, and addresses are replaced by band forms; names are
  stripped; `*_local` free text is excluded. The banded output is the *only*
  form an outbound context can be built from — `pipeline::run` is the sole
  construction path for `OutboundContext`, and it builds it from the redactor's
  output, never from caller data. A redaction failure is a BLOCK: nothing
  failing Layer 1 ever reaches Layer 2.
- **Layer 2 — semantic leak-scan** (`src/laya.rs`): the Layer 1 output is scanned
  on-device by the Laya sidecar (path 2 below) for six leak classes, including
  `unique_combination` — fields individually harmless that single out one family
  together.
- **Layer 3 — fail-closed router** (`ha-core`, `PolicyConfig`): plain-Rust match
  over the scan report. Flagged → one quarantine retry (shed verbatim text,
  re-redact, rescan) → still flagged, scanner unavailable, or confidence below
  floor → BLOCK. The model judges; this code decides; nothing sends on
  uncertainty.
- **The receipt** (`src/egress.rs`, `egress_log` table): every attempt — allowed
  or blocked — writes an append-only receipt (payload, SHA-256, both verdicts,
  decision) *before* the verdict is returned. The family-facing privacy screen
  reads only this table; no UPDATE or DELETE path exists (enforced by trigger).

**The send step, stated precisely:** the gate never holds a socket.
`GateVerdict::Allowed` *authorizes* the caller to transmit — the caller holds
the socket. Today, no such caller exists anywhere in the shipped code: the CLI's
`research` command runs the full pipeline and prints the receipt, and transmits
nothing; the app crate's IPC surface is a placeholder command. The first
external send will ship only behind this gate, and this section will be updated
to name the sending module when it exists.

**Residual risk:** Rust has no capability system that makes "no socket outside
`ha-privacy`" memory-safe; it is a design invariant enforced by review — grepping
for socket-owning constructs outside the gate — and by the egress log, which
makes an ungated send *auditable after the fact* if never preventable in
principal. The workspace additionally forbids `unsafe` code (`unsafe_code =
"forbid"`), so the send path cannot smuggle in FFI networking.

### 2. Laya leak-scan — loopback by construction

The semantic scan runs as a local sidecar process; the client
(`crates/ha-privacy/src/laya.rs`) is the only network client in the workspace.

- The sidecar URL is **loopback-only by construction**: the constructor rejects
  any non-loopback host — parsed as an IP, not by string prefix, so
  `127.0.0.1.evil.com` is refused; plain `http` only; `0.0.0.0` refused. The
  client cannot be pointed at a cloud endpoint even by mistake.
- The payload the sidecar sees is **Layer 1 output** — generalized bands, never
  raw family data. The checker sees the already-redacted form.
- One `POST /predict` per scan, all six leak classes in one forward pass, 5 s
  timeout so a hung sidecar blocks egress instead of stalling the gate.
- Failure mapping is fail-closed: unreachable, timed out, non-200, unparseable,
  or probabilities outside `0..=1` (a NaN must never reach the router, where
  `NaN > threshold` is false and a leak would read as clean) → `ScanError` →
  router BLOCK with a receipt.

**Residual risk:** another local process could bind the sidecar's port and
answer "clean" to scan requests. That attacker is already on the device (outside
this model's scope, per the assumption above) and cannot exfiltrate anything by
itself — the send step still requires an ALLOW from the gate.

### 3. Laya model download — user-initiated, outside the app binary

The sidecar's model checkpoint (`convaiinnovations/laya`, ~650 MB–2.3 GB) is
fetched from the Hugging Face Hub on first load — **by the user**, **by the
sidecar's own Python process**, never by the app binary and never on CI
(`crates/ha-privacy/MANUAL-SMOKE.md`). The app performs no model download; there
is no code path in the workspace that fetches model weights.

**Residual risk:** the download itself is an outbound request (to
huggingface.co) that a network observer can see. It carries family data only if
the user pastes data into it — it is a model-weights fetch. The user initiates
it explicitly.

### 4. Tauri IPC — in-process, no port

The desktop shell (`app/src-tauri`) has no HTTP listener: the frontend reaches
the Rust core exclusively through Tauri IPC, which is an in-process channel —
the app opens no port to listen on, locally or otherwise.

- The IPC surface is enumerated and auditable: every command is registered in
  `generate_handler!`, and the milestone-2 verification criteria require an IPC
  surface audit test proving no command accepts raw SQL, file paths, or network
  targets from the webview.
- Data access exists only in the Rust backend; the frontend renders state and
  never holds store or socket access.
- Production builds load the bundled frontend assets (`frontendDist`) from the
  app's own install directory — no remote content is loaded into the webview by
  the app.
- The dev server (`devUrl: http://localhost:5173`) exists only in `tauri dev`,
  binds loopback, and is not part of any built artifact.

**Residual risk:** the IPC bridge is the one place webview input reaches the
Rust core. The audit test and the thin-adapter rule (commands delegate to the
`ha-*` crates, no logic of their own) are the controls.

### 5. Third-party webview runtime — the declared boundary

The UI renders in whatever browser runtime the OS provides: **WebView2 on
Windows** — Microsoft's Edge-based runtime, which **sends telemetry to Microsoft
by default** — and WebKitGTK / WKWebView on Linux / macOS. The app cannot gate,
audit, or intercept the runtime's own traffic; a hard-coded CSP or IPC rule
cannot bind a vendor's browser process.

This is the declared boundary, named so the claim stays honest: the external
claim covers the app's own egress (paths 1–4, 6–7), not the webview runtime's.
It is also why the device-level absolute claim was retired rather than quietly
narrowed.

**Residual risk:** accepted and declared. Hardening (a strict CSP in
`tauri.conf.json` — currently unset — plus shrinking the IPC surface) reduces
what the runtime can be made to load, and is tracked below, but nothing in the
app can govern the vendor runtime's own connections.

### 6. Updates — user-initiated only

There is no updater. The Tauri updater plugin is not present, no update-check
endpoint is contacted, and the app never phones home to compare versions. Builds
ship as `deb`/`rpm` (and platform bundles later) that the user downloads and
installs — update traffic is ordinary package-manager traffic the user
initiates, from infrastructure the user chose to fetch from.

**Residual risk:** users on old versions get no nudge (accepted trade of safety
for silence); package integrity is the package manager's and the signing
process's job, out of this document's scope.

### 7. Dependencies — the zero-telemetry supply-chain policy

The app inherits the network behavior of everything it links. The policy:
**zero-telemetry dependencies** — no crate or npm package whose purpose is
telemetry, analytics, crash reporting, or ad SDKs; permissive licenses only;
known-vulnerable versions out; and code from crates.io only (no git or path
substitution slipping in).

The policy is enforced in CI by `cargo-deny` (`.github/workflows/ci.yml`, job
`cargo-deny`; configuration in `deny.toml`):

- **`bans`** — a deny-list of known telemetry/analytics/crash-reporting crates
  (the `sentry` family, `posthog`, `mixpanel`, `segment`, `amplitude`,
  `datadog`, `opentelemetry`, and kin). Adding a telemetry dep fails the audit;
  if one is ever genuinely needed, `deny.toml` is where that fight happens —
  explicitly, in review, never silently.
- **`licenses`** — an allowlist of known permissive licenses; anything unlisted
  fails the audit.
- **`advisories`** — RustSec security advisories against the locked graph fail
  the audit.
- **`sources`** — unknown registries and git sources are refused; the graph
  comes from crates.io only.

At the time of writing the lockfile holds 447 packages and the deny-list matches
none of them; the audit job running on every push and PR is the standing proof.

## Threat summary

| Threat                                        | Vector                              | Control                                                   | Residual                          |
|-----------------------------------------------|-------------------------------------|-----------------------------------------------------------|-----------------------------------|
| Buggy or rogue agent code egresses raw data   | Any send outside the gate           | Single gated pipeline; fail-closed router; receipts        | Review-enforced socket invariant  |
| Subtle leak a banded payload still allows     | `unique_combination` class          | Layer 2 scan + quarantine retry; confidence floor          | Model recall is imperfect — Layer 1 remains primary |
| Compromised or mis-pointed semantic scanner   | Sidecar HTTP client                 | Loopback-only construction; malformed-report refusal       | Local-port impersonator (on-device attacker — out of scope) |
| The webview runtime phones home               | Vendor browser process              | **None — declared boundary**                               | Accepted, named, and documented   |
| Telemetry arrives via a dependency            | Supply chain                        | `cargo-deny` bans, licenses, advisories, source pinning    | Zero-day crates not yet in RustSec |
| Silent auto-update phoning home               | Updater                             | No updater exists; user-initiated installs only            | Users on stale versions           |
| Device malware reads local data               | On-device attacker                  | **Out of scope** — declared assumption                     | Device security is the OS's domain |

## Known gaps and hardening backlog

Stated so the claim never exceeds the document:

1. **No CSP is configured** (the `app.security.csp` field is `null` in
   `tauri.conf.json`).
   The webview loads only bundled local content and the IPC surface is
   thin-adapter only, but a strict CSP is the right defense-in-depth and is
   tracked as a follow-up, not claimed as done.
2. **The socket invariant is review-enforced, not memory-safe** (path 1,
   residual risk). The egress log bounds the damage to *auditable*, not
   *impossible*. A capability-style build-time check is a candidate for a later
   milestone.
3. **Scan thresholds are code constants** in `PolicyConfig` pending calibration
   against labelled data (spec: "thresholds, chosen from labelled data — not
   0.5 defaults").
4. **The research-egress sender does not exist yet.** When it ships, this
   document gains a subsection naming the module that holds the socket and the
   tests that pin it.
