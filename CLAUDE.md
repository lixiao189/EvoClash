# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

EvoClash is a top-down 2D multiplayer shooter set in a post-apocalyptic wasteland (Rust, Bevy 0.20, lightyear 0.31 over WebTransport/netcode.io, axum matchmaker). The client runs only in the browser (wasm32). Gameplay rules are in `doc/gameplay.md`, architecture in `doc/architecture.md`, and the remaining crash-recovery work in `doc/plan.md`. README.md (in Chinese) keeps the quick start, deploy commands and release process.

## Git workflow (required)

Never commit or push directly to `main`. For every change:

1. Create a branch first.
2. Commit there and push the branch.
3. Open a PR against `main`.
4. Enable auto-fix and auto-merge on the PR.

Branch protection requires the single `CI success` check (it aggregates fmt, clippy (native + wasm), tests, cargo-deny, helm lint, and a Docker image build that runs the image and waits for a launched match to heartbeat).

## Commands

```bash
cargo build                                          # all crates; dev profile optimizes deps (Bevy is slow unoptimized)
cargo fmt --all --check                              # CI: rustfmt
cargo clippy --workspace --all-targets --locked -- -D warnings   # CI: warnings are errors
cargo clippy -p evoclash --target wasm32-unknown-unknown --locked -- -D warnings   # CI: the real client build
cargo test --workspace --locked                      # CI: tests
cargo test -p evoclash-game <test_name>              # one test in one crate (crates: evoclash-game, evoclash-server, evoclash, evoclash-matchmaker)
cargo deny --all-features check                      # CI: advisories / licenses / sources
helm lint --strict deploy/helm/evoclash               # CI: Helm chart
```

Run locally (needs `cargo install trunk`; `make web` adds the wasm32 target itself): `make run` builds the wasm client into `target/web` and starts `evoclash-matchmaker --web-dir target/web`, which serves it at `http://127.0.0.1:8080/play/` and launches match servers on demand (`make run LISTEN=host:port BOTS=n`, defaults 127.0.0.1:8080 and 6; local match-server output goes to `logs/`, cleared by `make clean-logs`). `make watch` rebuilds the client on change; `make check` runs the CI lints/tests. Page query params: `?name=` (required; index.html asks), `&lag=<ms>` simulates latency. A bare `evoclash-server --port 7000 --bots 4` still runs, but browsers can only reach a server through the matchmaker (they need its certificate hash). Server `--no-monsters` disables monsters.

The client crate still compiles natively so `cargo test`/clippy cover it (sprite preview tests), but its native `main` just exits; dead-code lints are relaxed natively and enforced by the wasm clippy run.

When asking the user to look at a client change, rebuild first (`make web`, then reload the page) — a stale build has caused confusion before.

## Architecture

The design write-up is `doc/architecture.md` (Chinese). Do not restate it here. Rules that are easy to break while editing:

- Change movement and combat rules in `crates/game/src/sim.rs` only. Do not change `sim::step_movement`'s signature unless every caller changes with it. Map generation stays on the SplitMix in `rng.rs` / `map/lot.rs` — no `rand`, no platform float functions — and must stay bit-identical on wasm and native.
- 64 Hz `FixedUpdate` is ordered `Input → Simulate → Combat → Resolve → Flow` (`SimSet` in `server/src/gameplay/mod.rs`). `FixedUpdate` stays single-threaded so a match log replays the same way it was recorded. Damage is queued and applied in `Resolve`; events flush from `Outbox` in `Flow`.
- New replicated components go through `ProtocolPlugin`. Bump `PROTOCOL_ID` in `net.rs` on any wire-format change.
- The client draws its own shots. The server does not echo them.
- Chinese UI is the bundled GB2312 Noto Sans SC subset. Do not regenerate the font unless a glyph is missing (`crates/client/assets/README.md`).
- Art files under `crates/client/assets/lpc/` and `lpc-src/` need a row in `assets/CREDITS.md`.
- `journal.rs` is past the ~600-line split threshold. Split it in its own PR before the crash-recovery work in `doc/plan.md`.

Workspace lints allow clippy `type_complexity` and `too_many_arguments` (normal for Bevy systems).

## Sub-agent team

The main session is the lead: it talks to the user, splits the work, briefs agents, merges their results, and alone does git (branch/commit/PR).

The same five roles live in two places. Pi discovers `.pi/agents/*.md` (`advertise: true`; aliases `策划` / `服务端` / `客户端` / `美术` / `测试`). Those agents inherit the session model; thinking is `medium` for `game-designer` and `artist`, `high` for `server-dev` and `client-dev`, `low` for `game-tester`. Claude Code uses `.claude/agents/`: designer, server, client, and artist on sonnet, tester on haiku.

| Agent | Owns | Use for |
|---|---|---|
| `game-designer` | `doc/gameplay.md` (no code) | rules, numbers, balance → short spec |
| `server-dev` | `crates/server`, `crates/matchmaker`, `crates/game`, `deploy/` | authoritative rules, protocol, bots, monsters, matchmaking |
| `client-dev` | `crates/client` except the art paths below | rendering, effects, HUD, input, prediction, session |
| `artist` | `crates/client/src/textures/`, `crates/client/src/lpc/`, art under `crates/client/assets/` (`lpc/`, `lpc-src/`, credits) | LPC sheets, procedural sprites, poses, art credits |
| `game-tester` | `#[cfg(test)]` code only | fmt/clippy/tests, missing tests, failure triage |

Keep it cheap:

- Don't spawn for small things. One-file fixes, questions and quick reads stay in the main session; agents are for multi-file features or a long noisy step (a full CI run).
- Pipeline only as far as needed: `game-designer` only when rules/numbers are undecided → `server-dev` (shared `crates/game` first, since the client builds on it) → `client-dev` and `artist` → `game-tester`. `artist` settles a new `Textures` field or LPC frame API before `client-dev` draws it. Server and client may run in parallel only when the protocol is already settled and they touch no shared file; client and artist may run in parallel only on disjoint files.
- Briefs are short and self-contained: goal, exact files/symbols, acceptance check, and the spec lines that matter — not the whole conversation. Agents already get this file, so don't repeat it.
- Follow up with the agent that has the context (Pi: resume that child; Claude Code: `SendMessage`) instead of spawning a fresh one.
- Agents run scoped, quiet checks (`-p <crate>`, `-q`, `--message-format=short`); the full `make check` runs once, in `game-tester`, before the PR.
- Reports are capped (≈15 lines, file:line refs, no code dumps); relay only what the user needs.

Avoid edit conflicts between agents:

- Split work by file ownership, not by feature step. Every brief lists the files the agent may edit; agents running at the same time never share a file. Agents edit only their files and put any change needed elsewhere in their report for the lead to apply.
- Hub files have one writer: `crates/game/src/protocol.rs` and `PROTOCOL_ID` in `net.rs`, plugin registration (`build` in `render/mod.rs`, `gameplay/mod.rs`, `monsters/mod.rs`, both `main.rs`/`config.rs`), `textures/mod.rs` (`Textures` fields and `build`, `artist`) and `lpc/mod.rs` (sheet layout, `artist`), `Cargo.toml`/`Cargo.lock`, and docs (`CLAUDE.md`, `README.md`, `doc/`, `.claude/`, `.pi/`). The lead (or one named agent) makes these edits first, before fanning out, so later agents build against a settled interface.
- Big areas are module directories (`client/src/render/`, `client/src/textures/`, `game/src/map/`, `server/src/gameplay/`, `server/src/monsters/`): `mod.rs` holds shared types, constants and the plugin; each child file is one concern (`use super::*;`, items `pub(super)`). Put new code in the child it belongs to, or a new child file, rather than growing `mod.rs`, so parallel agents land in different files.
- Each such directory's `tests.rs` (and other `#[cfg(test)]` code) belongs to `game-tester` while implementers run in parallel. Ignored art previews and the LPC bake test stay with `artist`; do not let both edit the same `tests.rs`.
- Agents sharing this checkout may run together only with disjoint file sets. If overlap can't be avoided, run them one after another, or give each `isolation: "worktree"` and the lead merges the branches.
- When a file passes ~600 lines, or two tasks collided in it, split it into a module directory as above in its own refactor PR before the feature work.

Escalate the model per call (Claude Code: `model: "opus"`; Pi: a stronger `model` from that session's registry), not by editing the agent files, when a task touches:

- `sim.rs` or client prediction/rollback — a mismatch doesn't fail the build, it shows up as rubber-banding under lag;
- lag compensation (`combat.rs` position history) or `SimSet` ordering (when damage and events land);
- map generation (`map/`, `nav.rs`) — must stay bit-identical across wasm/native and pass the seed sweeps;
- unfamiliar Bevy 0.20 / lightyear 0.31 APIs, where a weaker model guesses old signatures;
- one feature spanning protocol + server + client.

Alternatively the lead writes the interface/design for such work, a default implementer writes it, and the lead reviews the diff. Also escalate (don't retry) when an agent fails the same build or test twice, or comes back with a wrong or vague result — a few failed default-model rounds cost more than one stronger run. If `game-tester` misreads failures, rerun it on a stronger model.

## Releases

Bump `workspace.package.version` in `Cargo.toml` and `version`/`appVersion` in `deploy/helm/evoclash/Chart.yaml`, merge it, then push a matching tag `vX.Y.Z` (tags containing `-` are pre-releases). `release.yml` checks that the tag matches both versions, builds the web client (the only release asset, `evoclash-web-vX.Y.Z.tar.gz`) and the Linux server/matchmaker binaries that only go into the image (with `web/` bundled), pushes `ghcr.io/lixiao189/evoclash-server`, then pushes the chart to `oci://ghcr.io/lixiao189/charts/evoclash`.
