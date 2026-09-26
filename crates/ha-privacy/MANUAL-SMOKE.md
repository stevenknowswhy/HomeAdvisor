# MANUAL-SMOKE: the real laya-serve sidecar (optional — never on CI)

The sidecar contract tests in `tests/laya_sidecar.rs` run against a mock HTTP
server on every CI run. This smoke test is the one leg CI cannot run: it
exercises `LayaSidecar` against a **real** `laya-serve` sidecar process.

**Why it stays off CI:** the English checkpoint downloads ~2.3 GB from the
Hugging Face Hub on first load (a `subfolder` checkpoint, 650–850 MB). CI
never installs `laya` or downloads model weights.

## 1. Install and start the sidecar

Upstream's maintained `laya-serve` server replaces the hand-written FastAPI
wrapper this repo used to document — there is no sidecar script to copy
anymore. The pinned version is **`laya[serve]==0.3.20`**; upstream releases
move fast, so review the `/v1/systemone` contract before moving the pin
(the client and the mock fixtures are the contract's other half).

```sh
python3 -m venv .laya-venv && source .laya-venv/bin/activate
pip install "laya[serve]==0.3.20"

# The four pins are deliberate:
LAYA_HOST=127.0.0.1 \        # the bare-metal default is 0.0.0.0 — never inherit it
LAYA_PORT=8000 \             # the port DEFAULT_SIDECAR_URL and HA_LAYA_URL expect
LAYA_MODELS=english \        # preload one checkpoint, not three (multi-GB each)
LAYA_PRELOAD=1 \             # load before listening, so a healthy /health implies ready
LAYA_DEVICE=cpu \            # mps on Apple Silicon (the distribution target); cpu is the documented tail
laya-serve
```

The model loads once at startup (25–35 s); with `LAYA_PRELOAD=1` the server
starts listening only after the checkpoint is in memory. `GET /health` is
the readiness signal the app's supervisor probes:

```text
{"status":"ok","loaded":["english"],"device":"cpu"}
```

Tips from upstream: if the weight download hangs at 0 bytes, set
`HF_HUB_DISABLE_XET=1` and retry; once cached, `HF_HUB_OFFLINE=1` skips the
network check.

## 2. Run the smoke test

```sh
cargo test -p ha-privacy --test laya_sidecar real_laya_sidecar_smoke -- --ignored --nocapture
```

- `--ignored` selects the smoke test alone; the mock-server tests always run
  without it. The test **compiles** on CI but never runs there.
- Override the sidecar address with `HA_LAYA_URL` (default
  `http://127.0.0.1:8000`). It must be loopback — `LayaSidecar::new` rejects
  any other host, by construction.

## 3. What passing looks like

The test asserts structure only — the model's calibration is a tuning
concern (thresholds live in `ha_core::PolicyConfig`), not the client's:

- the sidecar is reachable and answers `/v1/systemone`;
- one forward pass returns **all six** leak classes
  (`full_name`, `exact_dob`, `named_place`, `street_address`, `gov_id`,
  `unique_combination`);
- every probability (and the overall confidence) is in 0..=1.

`--nocapture` prints the parsed `ScanReport`, e.g.:

```text
real Laya scan report: ScanReport {
    per_class: [(FullName, 0.01), (ExactDob, 0.01), (NamedPlace, 0.02),
                (StreetAddress, 0.01), (GovId, 0.01), (UniqueCombination, 0.04)],
    confidence: 0.93,
}
```

On this banded payload every class should sit near zero — it contains no
PII by construction. Warm calls run 20–35 ms; a scan of six questions is one
round trip.

### The question shape (upstream issue #156)

The English checkpoint can follow its option labels instead of the state and
return a confident "no" for clearly positive input — for a leak scan, that
is the wrong-direction failure. The client therefore sends every `noul`
question with `criteria` keyed `true`/`false` and a neutral `A`/`B` labels
override, per upstream's documented workaround; `laya-serve` 0.3.20 answers
with `noul` = P(true) either way. A real-sidecar validation run (banded
payload clean, leaky payload flagged) is recorded in the migration PR.
