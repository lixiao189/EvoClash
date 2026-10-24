# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

EvoClash is a top-down 2D multiplayer shooter set in a post-apocalyptic wasteland (Rust, Bevy 0.20, lightyear 0.31 over WebTransport/netcode.io, axum matchmaker). The client runs only in the browser (wasm32). README.md (in Chinese) documents gameplay rules, CLI flags, HA/latency design and the release process.

## Git workflow (required)

Never commit or push directly to `main`. For every change:

1. Create a branch first.
2. Commit there and push the branch.
3. Open a PR against `main`.
4. Enable auto-fix and auto-merge on the PR.

Branch protection requires the single `CI success` check (it aggregates fmt, clippy, the 3-OS test matrix, cargo-deny, helm lint, and a Docker image build that runs the image and waits for a launched match to heartbeat).

## Commands

```bash
cargo build                                          # all crates; dev profile optimizes deps (Bevy is slow unoptimized)
cargo fmt --all --check                              # CI: rustfmt
cargo clippy --workspace --all-targets --locked -- -D warnings   # CI: warnings are errors
cargo clippy -p evoclash --target wasm32-unknown-unknown --locked -- -D warnings   # CI: the real client build
cargo test --workspace --locked                      # CI: tests
cargo test -p evoclash-game <test_name>              # one test in one crate (crates: evoclash-game, evoclash-server, evoclash, evoclash-matchmaker)
cargo deny check --all-features                      # CI: advisories / licenses / sources
helm lint --strict deploy/helm/evoclash               # CI: Helm chart
```

Run locally (needs `trunk`): `make run` builds the wasm client into `target/web` and starts `evoclash-matchmaker --web-dir target/web`, which serves it at `http://127.0.0.1:8080/play/` and launches match servers on demand. `make watch` rebuilds the client on change; `make check` runs the CI lints/tests. Page query params: `?name=` (required; index.html asks), `&lag=<ms>` simulates latency. A bare `evoclash-server --port 7000 --bots 4` still runs, but browsers can only reach a server through the matchmaker (they need its certificate hash). Server `--no-monsters` disables monsters.

The client crate still compiles natively so `cargo test`/clippy cover it (sprite preview tests), but its native `main` just exits; dead-code lints are relaxed natively and enforced by the wasm clippy run.

When asking the user to look at a client change, rebuild first (`make web`, then reload the page) — a stale build has caused confusion before.

## Architecture

Workspace crates:

- `crates/game` (`evoclash-game`) — shared by server and client: `protocol.rs` (replicated components, `PlayerInput`, `GameEvent` messages, channel registration in `ProtocolPlugin`), `sim.rs` (deterministic movement/combat rules), `map.rs`/`nav.rs` (arena + pathfinding), `items.rs` (weapon/armor tables, loot rolls), `net.rs` (tick rate, `PROTOCOL_ID`, netcode key loading), and `api` (matchmaker JSON contract) in `lib.rs`.
- `crates/server` (`evoclash-server`) — headless authoritative server, one process per match. Plugins: connections, gameplay, combat (lag compensation via position history), bots, monsters, ops (heartbeat/drain).
- `crates/client` (`evoclash`) — browser client: input, local prediction (`predict.rs`), rendering, effects, HUD, session/reconnect (async HTTP via `ehttp`, WebTransport link). `index.html` + `Trunk.toml` are the web shell; the UI font is a bundled Noto Sans SC subset (`assets/README.md` explains regenerating it when adding Chinese strings outside GB2312).
- `crates/matchmaker` (`evoclash-matchmaker`) — seat reservation, signed connect tokens (`tokens.rs`), match registry fed by heartbeats (`registry.rs`), local process launching (`fleet.rs`; `--no-launch` only adopts externally started servers), and serving the web client at `/play/` (`--web-dir`, default `web/` next to the binary).

Key cross-cutting points:

- **Shared determinism.** Movement must stay identical on both sides: the server runs `sim` authoritatively and the client re-runs the same functions for prediction/rollback of the local player. Change rules in `crates/game/src/sim.rs`, not separately in server or client.
- **Server tick.** 64 Hz `FixedUpdate`, ordered by the chained `SimSet` sets in `server/src/gameplay.rs`: `Input → Simulate → Combat → Resolve → Flow`. Damage is queued (`DamageQueue`) and applied in `Resolve`; outgoing events are buffered in an `Outbox` and flushed in `Flow`. New gameplay systems should go into the right set.
- **Replication.** State replicates every 2 ticks (`REPLICATION_INTERVAL`). New replicated components must be registered in `ProtocolPlugin`. Bump `PROTOCOL_ID` in `net.rs` on any wire-format change so stale clients are rejected at handshake.
- **Own shots are client-drawn.** The client draws its own projectiles immediately; the server does not echo them back and decides hits itself.
- **WebTransport certificates.** Each server makes a 14-day self-signed cert at startup (`TlsIdentity` in `connections.rs`) and reports its SHA-256 in heartbeats; the matchmaker passes it to browsers in `JoinResponse.cert_digest` (a join to a just-launched match waits for the first heartbeat). Servers drain themselves after 12 days. Browsers only allow WebTransport from secure pages, so anything beyond localhost needs HTTPS in front of the matchmaker.
- **Matchmaker ↔ server.** Servers heartbeat every 2 s (players, `tick_us`, cert digest) to every matchmaker URL, expanding hostnames to all resolved addresses (headless Service → all replicas); a server missing heartbeats for ~6 s is dropped. Match processes outlive a matchmaker restart and are re-adopted via heartbeats. SIGTERM drains a server (finish the match, then exit).
- **Kubernetes.** The Helm chart `deploy/helm/evoclash` (check with `helm lint` / `helm template`) runs the matchmaker as a `--no-launch` Deployment and match servers as an Agones Fleet; `server/src/agones.rs` handles Ready/Health/self-Allocate/Shutdown over the SDK sidecar's REST API. Production needs `EVOCLASH_NETCODE_KEY` (64 hex chars) shared by matchmaker and servers; otherwise a dev key is used.
- **Procedural art, no assets.** `client/src/textures.rs` paints every sprite at startup into a `Canvas` (drawn at 2x, nearest sampling, characters face +X). `#[ignore]` preview tests write BMP contact sheets for checking art without running the game, e.g. `EVOCLASH_SPRITE_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_survivor` (see the tests module for others).
- **Depth.** `render.rs` sorts by `depth(y)`; parts of one actor are layered with tiny `Z_STEP` offsets relative to the actor.

Workspace lints allow clippy `type_complexity` and `too_many_arguments` (normal for Bevy systems).

## Releases

Bump `workspace.package.version` in `Cargo.toml` and `version`/`appVersion` in `deploy/helm/evoclash/Chart.yaml`, merge it, then push a matching tag `vX.Y.Z` (tags containing `-` are pre-releases). `release.yml` checks that the tag matches both versions, builds the web client once and the server/matchmaker binaries for each platform (each archive and the image bundle `web/`), pushes `ghcr.io/lixiao189/evoclash-server`, then pushes the chart to `oci://ghcr.io/lixiao189/charts/evoclash`.
