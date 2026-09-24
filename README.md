# Home Advisor

A local-first, privacy-gated family advisor. Home Advisor turns a private, on-device
family profile into a few sharp, evidence-backed actions a day across health, wealth,
education, career, lifestyle, and family happiness — for adults and children.

Trust is the product. Family data stays on the user's machine, and nothing that could
ever leave the device passes through a single, testable privacy gate.

## Status: milestone 1 — foundation

Building the two load-bearing pieces first, per the [Rust MVP spec](https://app.obvious.ai/p/prj_Y4WxY6M6?blueprint=art_1Ixvz0wL):

- **The on-device data model** — SQLite encrypted at rest (SQLCipher), band-typed PII,
  `*_local` free-text quarantine, sponsorship projected out of ranking.
- **The privacy gate** — deterministic redaction → local semantic leak-scan →
  fail-closed policy router, with an append-only egress receipt for every decision.

**In scope:** workspace crates, encrypted schema, the gate, receipts, a CLI demo, CI.
**Not yet:** GUI, agents, research integration, LLM integration, vendor marketplace —
those are later milestones that build on this gate, not alongside it.

## Workspace layout

| Crate        | Role |
|--------------|------|
| `ha-core`    | Domain types and the deterministic fail-closed policy router |
| `ha-store`   | Encrypted on-device persistence (rusqlite + SQLCipher) and the egress log |
| `ha-privacy` | The gate: deterministic redaction, semantic leak-scan, fail-closed vetting |
| `ha-cli`     | Demo surface: seeds a family, exercises the gate, prints the receipt |

The crates compile as scaffolding stubs today; milestone-1 tasks fill in types,
schema, and the gate.

## Development

Requires a stable Rust toolchain (`rustup` reads `rust-toolchain.toml`):

```sh
cargo build                                            # compile the workspace
cargo test                                             # run all tests
cargo clippy --workspace --all-targets -- -D warnings  # lint gate
```

CI runs the same three on every push and pull request.
