# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

EvoClash is a top-down 2D multiplayer shooter set in a post-apocalyptic wasteland (Rust, Bevy 0.20, lightyear 0.31 over UDP/netcode.io, axum matchmaker). README.md (in Chinese) documents gameplay rules, CLI flags, HA/latency design and the release process.

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
cargo test --workspace --locked                      # CI: tests
cargo test -p evoclash-game <test_name>              # one test in one crate (crates: evoclash-game, evoclash-server, evoclash, evoclash-matchmaker)
cargo deny check --all-features                      # CI: advisories / licenses / sources
helm lint --strict deploy/helm/evoclash               # CI: Helm chart
```

Run locally (direct connect, dev netcode key):

```bash
cargo run -p evoclash-server -- --port 7000 --bots 4
cargo run -p evoclash -- --server 127.0.0.1:7000 --id 1
```

Full path through the matchmaker (it launches match processes on demand): `evoclash-matchmaker --listen 127.0.0.1:8080`, then `evoclash --matchmaker http://127.0.0.1:8080 --name <name>`. Headless smoke test: `evoclash --server 127.0.0.1:7000 --headless --autopilot`. Client `--fake-latency <ms>` simulates lag; server `--no-monsters` disables monsters.

When asking the user to look at a client change, rebuild first — a stale `target/debug/evoclash` has caused confusion before.

## Architecture

Workspace crates:

- `crates/game` (`evoclash-game`) — shared by server and client: `protocol.rs` (replicated components, `PlayerInput`, `GameEvent` messages, channel registration in `ProtocolPlugin`), `sim.rs` (deterministic movement/combat rules), `map.rs`/`nav.rs` (arena + pathfinding), `items.rs` (weapon/armor tables, loot rolls), `net.rs` (tick rate, `PROTOCOL_ID`, netcode key loading), and `api` (matchmaker JSON contract) in `lib.rs`.
- `crates/server` (`evoclash-server`) — headless authoritative server, one process per match. Plugins: connections, gameplay, combat (lag compensation via position history), bots, monsters, ops (heartbeat/drain).
- `crates/client` (`evoclash`) — input, local prediction (`predict.rs`), rendering, effects, HUD, session/reconnect.
- `crates/matchmaker` (`evoclash-matchmaker`) — seat reservation, signed connect tokens (`tokens.rs`), match registry fed by heartbeats (`registry.rs`), local process launching (`fleet.rs`; `--no-launch` only adopts externally started servers).

Key cross-cutting points:

- **Shared determinism.** Movement must stay identical on both sides: the server runs `sim` authoritatively and the client re-runs the same functions for prediction/rollback of the local player. Change rules in `crates/game/src/sim.rs`, not separately in server or client.
- **Server tick.** 64 Hz `FixedUpdate`, ordered by the chained `SimSet` sets in `server/src/gameplay.rs`: `Input → Simulate → Combat → Resolve → Flow`. Damage is queued (`DamageQueue`) and applied in `Resolve`; outgoing events are buffered in an `Outbox` and flushed in `Flow`. New gameplay systems should go into the right set.
- **Replication.** State replicates every 2 ticks (`REPLICATION_INTERVAL`). New replicated components must be registered in `ProtocolPlugin`. Bump `PROTOCOL_ID` in `net.rs` on any wire-format change so stale clients are rejected at handshake.
- **Own shots are client-drawn.** The client draws its own projectiles immediately; the server does not echo them back and decides hits itself.
- **Matchmaker ↔ server.** Servers heartbeat every 2 s (players, `tick_us`) to every matchmaker URL, expanding hostnames to all resolved addresses (headless Service → all replicas); a server missing heartbeats for ~6 s is dropped. Match processes outlive a matchmaker restart and are re-adopted via heartbeats. SIGTERM drains a server (finish the match, then exit).
- **Kubernetes.** The Helm chart `deploy/helm/evoclash` (check with `helm lint` / `helm template`) runs the matchmaker as a `--no-launch` Deployment and match servers as an Agones Fleet; `server/src/agones.rs` handles Ready/Health/self-Allocate/Shutdown over the SDK sidecar's REST API. Production needs `EVOCLASH_NETCODE_KEY` (64 hex chars) shared by matchmaker and servers; otherwise a dev key is used.
- **Procedural art, no assets.** `client/src/textures.rs` paints every sprite at startup into a `Canvas` (drawn at 2x, nearest sampling, characters face +X). `#[ignore]` preview tests write BMP contact sheets for checking art without running the game, e.g. `EVOCLASH_SPRITE_PREVIEW=out.bmp cargo test -p evoclash -- --ignored preview_survivor` (see the tests module for others).
- **Depth.** `render.rs` sorts by `depth(y)`; parts of one actor are layered with tiny `Z_STEP` offsets relative to the actor.

Workspace lints allow clippy `type_complexity` and `too_many_arguments` (normal for Bevy systems).

## Releases

Bump `workspace.package.version` in `Cargo.toml` and `version`/`appVersion` in `deploy/helm/evoclash/Chart.yaml`, merge it, then push a matching tag `vX.Y.Z` (tags containing `-` are pre-releases). `release.yml` checks that the tag matches both versions, builds the binaries for each platform, pushes `ghcr.io/lixiao189/evoclash-server`, then pushes the chart to `oci://ghcr.io/lixiao189/charts/evoclash`.
