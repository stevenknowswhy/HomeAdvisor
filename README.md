# Home Advisor

A local-first, privacy-gated family advisor. Home Advisor turns a private, on-device
family profile into a few sharp, evidence-backed actions a day across health, wealth,
education, career, lifestyle, and family happiness — for adults and children.

Trust is the product. Family data stays on the user's machine, and nothing that could
ever leave the device passes through a single, testable privacy gate.

## Download & install (macOS, Apple Silicon)

The app is unsigned — no Apple Developer certificate yet — so whether Gatekeeper
says anything on first launch depends on how the DMG reaches your machine.

### Recommended: download with curl (zero Gatekeeper prompts)

```sh
curl -L -o ~/Downloads/HomeAdvisor.dmg https://github.com/stevenknowswhy/HomeAdvisor/releases/latest/download/HomeAdvisor_0.1.0_aarch64.dmg
```

Then open the downloaded image and drag Home Advisor to **Applications**. This
works with zero prompts because files fetched by curl carry no quarantine flag,
so Gatekeeper never intervenes. The `releases/latest/download` URL always
resolves to the newest published release asset as long as the naming scheme
`HomeAdvisor_<version>_aarch64.dmg` is kept.

### Alternative: browser download (quarantined — one Terminal step)

Download `HomeAdvisor_<version>_aarch64.dmg` (for v0.1.0:
`HomeAdvisor_0.1.0_aarch64.dmg`) from the
[latest release](https://github.com/stevenknowswhy/HomeAdvisor/releases/latest),
open the image, and drag Home Advisor to **Applications**. Browser-downloaded
files are quarantined, and because the app is unsigned, macOS may report it as
"damaged" on first launch. Strip the quarantine and open again:

```sh
xattr -cr /Applications/HomeAdvisor.app
```

If macOS still refuses to launch the app, re-sign it ad-hoc:

```sh
codesign --force --deep --sign - /Applications/HomeAdvisor.app
```

(Historical note: right-clicking the app and choosing **Open** — the classic
Gatekeeper bypass — no longer works for unsigned, quarantined apps on current
macOS, which reports them as damaged instead of offering the open-anyway
option.)

Signed and notarized builds are planned. The privacy claim the app makes is
scoped to its own egress, as [docs/threat-model.md](docs/threat-model.md)
defines it: the app never sends your PII anywhere — every outbound byte is
gated, redacted, and receipted.

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

The milestone-1 pieces are in place. `ha-cli` demonstrates the full round
trip — see [the CLI walkthrough](docs/cli-round-trip.md):

```sh
HA_STORE_KEY=<passphrase> cargo run -p ha-cli -- seed     --db demo.db
HA_STORE_KEY=<passphrase> cargo run -p ha-cli -- research --db demo.db --sidecar http://127.0.0.1:8000
HA_STORE_KEY=<passphrase> cargo run -p ha-cli -- receipt  --db demo.db
```

## Development

Requires a stable Rust toolchain (`rustup` reads `rust-toolchain.toml`):

```sh
cargo build                                            # compile the workspace
cargo test                                             # run all tests
cargo clippy --workspace --all-targets -- -D warnings  # lint gate
```

CI runs the same three on every push and pull request.
