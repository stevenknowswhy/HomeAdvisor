# MANUAL-SMOKE: the real Laya sidecar (optional — never on CI)

The sidecar contract tests in `tests/laya_sidecar.rs` run against a mock HTTP
server on every CI run. This smoke test is the one leg CI cannot run: it
exercises `LayaSidecar` against a **real** Laya sidecar process.

**Why it stays off CI:** the English checkpoint downloads ~2.3 GB from the
Hugging Face Hub on first load (a `subfolder` checkpoint, 650–850 MB). CI
never installs `laya` or downloads model weights.

## 1. Install and start the sidecar

```bash
python3 -m venv .laya-venv && source .laya-venv/bin/activate
pip install laya fastapi uvicorn
```

Save this as `laya_sidecar.py` — the integration guide's minimal sidecar,
plus the warmup call it recommends (the first call at a batch shape compiles
kernels; one throwaway predict keeps that off the measured path):

```python
import threading

import laya
from fastapi import FastAPI

app, lock = FastAPI(), threading.Lock()
agent = laya.load("convaiinnovations/laya")  # English checkpoint, 421M
agent.predict(
    "warmup",
    {"check": {"type": "noul", "instructions": "Is this a warmup check?"}},
)

@app.post("/predict")
def predict(body: dict):
    with lock:  # one forward pass at a time
        return agent.predict(body["state"], body["questions"])
```

```bash
uvicorn laya_sidecar:app --host 127.0.0.1 --port 8000
```

The model loads once at startup (25–35 s). Tips from the integration guide:
if the weight download hangs at 0 bytes, set `HF_HUB_DISABLE_XET=1` and retry;
once cached, `HF_HUB_OFFLINE=1` skips the network check.

## 2. Run the smoke test

```bash
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

- the sidecar is reachable and answers `/predict`;
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
