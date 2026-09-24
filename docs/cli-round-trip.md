# The CLI round trip, end to end

`ha-cli` is the milestone-1 proof of life: one binary that shows the whole
privacy pipeline working on device — a family profile enters, a
purpose-limited research request rides the gate, and a receipt lands in an
append-only log the family can read.

Three commands, run in order against one encrypted database:

```
HA_STORE_KEY=<passphrase> cargo run -p ha-cli -- seed     --db demo.db
HA_STORE_KEY=<passphrase> cargo run -p ha-cli -- research --db demo.db --sidecar http://127.0.0.1:8000
HA_STORE_KEY=<passphrase> cargo run -p ha-cli -- receipt  --db demo.db
```

The key lives in the environment for one invocation, never in a file, never
beside the database. The sidecar URL must be loopback — the semantic scan
never leaves this machine.

## 1. `seed` — the onboarding moment

The demo family's facts (two adults, a child, an address, an income, three
goals) enter memory, and the store persists **generalized bands only**:
age bands, an income-band foreign key, a region class, quarantined
`*_local` free text. The exact address and figures are used to pick the
bands and are gone when the command returns. Seeding twice is an error, not
a duplicate family.

## 2. `research` — the gate round trip

The command assembles an outbound draft the way a live agent would: raw
profile facts from memory plus wealth-domain goals read back from the
store. Purpose limitation is structural — children's names and school
details are never drafted, and every field the draft does carry must be
named by the redaction plan.

Then the three layers run:

1. **Layer 1 (deterministic):** the allowlist walker strips name fields,
   generalizes ages → bands, income → band, and the street address → a
   region class. Unruled paths are removed and recorded — the default is
   no egress.
2. **Layer 2 (semantic):** the real `LayaSidecar` client posts the banded
   payload as `state` to the loopback `/predict` endpoint — one forward
   pass, all six leak questions at once. The sidecar sees generalized
   payloads, never raw family data.
3. **Layer 3 (policy router):** plain Rust code decides. Clean scan with
   adequate confidence → ALLOW. Anything flagged → one re-generalization
   pass (the demo sheds its verbatim free text) and a rescan; still
   flagged, uncertain, or unreachable → BLOCK. The gate fails closed,
   never open.

Either way, the screen prints what happened — and the exit code says so
for scripts: blocked requests exit non-zero.

## 3. `receipt` — the family-facing privacy screen

Reads the append-only `egress_log`: every attempt to move family data off
the device, allowed or blocked, with the exact payload bytes, their SHA-256
hash, both verdicts, and the decision. The screen's only source is that
log — there is no other window onto egress.

## Running it without the model

The sidecar only needs to speak the `/predict` contract; the 650 MB–2.3 GB
model download belongs off CI. See `crates/ha-privacy/MANUAL-SMOKE.md` for
the real-model smoke test, and `crates/ha-privacy/tests/laya_sidecar.rs`
for the contract shape a stand-in sidecar must serve. With no sidecar
reachable, `research` still completes — as a BLOCK with a receipt — which
is the fail-closed design working, not a crash.

## What the tests pin

`crates/ha-cli/tests/cli_round_trip.rs` runs the exact commands a user
runs, against a loopback mock:

- A seeded family yields a gated payload containing no raw PII strings —
  banded forms only — and the sidecar saw the banded payload, not the raw
  draft.
- The printed receipt matches the `egress_log` row: same receipt id, same
  payload bytes, same hash, same decision.
- An unreachable sidecar blocks the request and still leaves a receipt.
